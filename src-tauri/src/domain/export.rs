use crate::domain::summary::MeetingInsights;
use crate::domain::transcript::LiveTranscript;
use docx_rs::{Docx, Paragraph, Run};
use printpdf::{BuiltinFont, Mm, PdfDocument};
use std::io::BufWriter;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Pdf,
    Docx,
}

impl ExportFormat {
    pub fn from_ext(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Docx),
            _ => None,
        }
    }
}

/// Build canonical markdown for a meeting export.
pub fn build_markdown(
    title: &str,
    transcript: &LiveTranscript,
    insights: Option<&MeetingInsights>,
) -> String {
    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n"));
    if let Some(i) = insights {
        if !i.summary.is_empty() {
            md.push_str("## Summary\n\n");
            md.push_str(&i.summary);
            md.push_str("\n\n");
        }
        if !i.key_points.is_empty() {
            md.push_str("## Key points\n\n");
            md.push_str(&i.key_points_text());
            md.push_str("\n\n");
        }
        if !i.action_items.is_empty() {
            md.push_str("## Action items\n\n");
            md.push_str(&i.action_items_text());
            md.push_str("\n\n");
        }
    }
    md.push_str("## Transcript\n\n");
    for seg in transcript.segments() {
        md.push_str(&format!(
            "**{}** ({}): {}\n\n",
            seg.speaker.label(),
            crate::domain::transcript::format_ts(seg.start_ms),
            seg.text
        ));
    }
    md
}

pub fn export_markdown_to_path(path: &Path, markdown: &str) -> std::io::Result<()> {
    std::fs::write(path, markdown)
}

pub fn export_pdf_to_path(path: &Path, title: &str, body: &str) -> Result<(), String> {
    let (doc, page1, layer1) = PdfDocument::new(title, Mm(210.0), Mm(297.0), "Layer 1");
    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| e.to_string())?;
    let layer = doc.get_page(page1).get_layer(layer1);

    let mut y = 280.0;
    layer.use_text(title, 16.0, Mm(15.0), Mm(y), &font);
    y -= 12.0;

    for line in body.lines() {
        if y < 15.0 {
            break;
        }
        let clipped: String = line.chars().take(95).collect();
        layer.use_text(&clipped, 10.0, Mm(15.0), Mm(y), &font);
        y -= 5.0;
    }

    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut buf = BufWriter::new(file);
    doc.save(&mut buf).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn export_docx_to_path(path: &Path, title: &str, body: &str) -> Result<(), String> {
    let mut docx =
        Docx::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text(title).bold()));
    for line in body.lines() {
        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(line)));
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    docx.build().pack(file).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn export_meeting(
    path: &Path,
    format: ExportFormat,
    title: &str,
    transcript: &LiveTranscript,
    insights: Option<&MeetingInsights>,
) -> Result<String, String> {
    let md = build_markdown(title, transcript, insights);
    match format {
        ExportFormat::Markdown => {
            export_markdown_to_path(path, &md).map_err(|e| e.to_string())?;
        }
        ExportFormat::Pdf => {
            export_pdf_to_path(path, title, &md)?;
        }
        ExportFormat::Docx => {
            export_docx_to_path(path, title, &md)?;
        }
    }
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::speaker::Speaker;
    use crate::domain::transcript::TranscriptSegment;
    use tempfile::tempdir;

    fn sample() -> (LiveTranscript, MeetingInsights) {
        let mut t = LiveTranscript::new();
        t.append(TranscriptSegment::new(Speaker::Me, "Hello team", 0, 1000));
        t.append(TranscriptSegment::new(
            Speaker::Others,
            "Hi there",
            1000,
            2000,
        ));
        let insights = MeetingInsights {
            summary: "Introductions".into(),
            key_points: vec!["Greetings exchanged".into()],
            action_items: vec!["Follow up tomorrow".into()],
        };
        (t, insights)
    }

    #[test]
    fn markdown_contains_all_sections() {
        let (t, i) = sample();
        let md = build_markdown("Standup", &t, Some(&i));
        assert!(md.contains("# Standup"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("Introductions"));
        assert!(md.contains("## Transcript"));
        assert!(md.contains("Me"));
        assert!(md.contains("Hello team"));
        assert!(md.contains("Action items"));
    }

    #[test]
    fn exports_all_formats_to_disk() {
        let dir = tempdir().unwrap();
        let (t, i) = sample();
        for (name, fmt) in [
            ("out.md", ExportFormat::Markdown),
            ("out.pdf", ExportFormat::Pdf),
            ("out.docx", ExportFormat::Docx),
        ] {
            let path = dir.path().join(name);
            export_meeting(&path, fmt, "Standup", &t, Some(&i)).unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            assert!(meta.len() > 10, "{name} should have content");
        }
    }

    #[test]
    fn format_from_ext() {
        assert_eq!(ExportFormat::from_ext("md"), Some(ExportFormat::Markdown));
        assert_eq!(ExportFormat::from_ext("PDF"), Some(ExportFormat::Pdf));
        assert_eq!(ExportFormat::from_ext("docx"), Some(ExportFormat::Docx));
        assert_eq!(ExportFormat::from_ext("txt"), None);
    }
}
