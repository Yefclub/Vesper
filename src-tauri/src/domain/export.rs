use crate::domain::speaker::SpeakerNames;
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

    /// The extension a suggested filename carries. Derived from the parsed
    /// format rather than echoed from the caller's string, so the name offered
    /// in the dialog is always one `from_ext` accepts back.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
        }
    }
}

/// Characters Windows refuses in a path component.
const RESERVED: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// MS-DOS device names, which are still not usable filenames, with or without
/// an extension: `CON.md` opens the console, not a file.
const DEVICE_NAMES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

/// NTFS counts 255 UTF-16 code units per path component and the caller appends
/// an extension. Counting units rather than chars is the honest measure: one
/// emoji is two of them.
const MAX_STEM_UNITS: usize = 240;

/// Turn a meeting title into a filename stem the OS will actually accept.
///
/// Five rule classes, not one, because each fails differently: a reserved char
/// is refused outright, a control char is refused by some APIs and silently
/// mangled by others, a trailing dot or space is **stripped silently** so the
/// dialog and the file on disk disagree about the name, a device name resolves
/// to hardware, and an over-long component is refused whole. A generated title
/// hits the first of these routinely — `Client call: Q3 budget` is exactly the
/// shape a model produces.
pub fn safe_file_stem(title: &str) -> String {
    // Reserved chars separate rather than substitute: `Client call: Q3 budget`
    // becomes `Client call - Q3 budget`, which still reads as a title, instead
    // of `Client call- Q3 budget`, which reads as a bug.
    let cleaned: String = title.chars().filter(|c| !c.is_control()).collect();
    let joined = cleaned
        .split(|c| RESERVED.contains(&c))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" - ");

    let mut stem = String::new();
    let mut units = 0usize;
    for ch in joined.chars() {
        let width = ch.len_utf16();
        if units + width > MAX_STEM_UNITS {
            break;
        }
        units += width;
        stem.push(ch);
    }

    // After the cut, not before: the cut can expose a trailing dot of its own.
    let stem = stem.trim_end_matches(['.', ' ']);
    if stem.is_empty() || is_device_name(stem) {
        return "meeting".into();
    }
    stem.into()
}

fn is_device_name(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or(stem).trim();
    let upper = base.to_ascii_uppercase();
    if DEVICE_NAMES.contains(&upper.as_str()) {
        return true;
    }
    // COM1-9 and LPT1-9; COM0 is not reserved.
    let bytes = upper.as_bytes();
    (upper.starts_with("COM") || upper.starts_with("LPT"))
        && bytes.len() == 4
        && bytes[3].is_ascii_digit()
        && bytes[3] != b'0'
}

/// Build canonical markdown for a meeting export.
/// The heading an exported section is written under.
///
/// The templates' own English, found by key across all of them, so the exported
/// file reads the way the model was asked to write it. An unknown key — a
/// document written by a build whose templates have since changed — is titled
/// from the key itself rather than dropped: an export that silently omits a
/// section is worse than one with an ugly heading.
fn section_heading(key: &str) -> String {
    use crate::domain::summary::SummaryTemplate;
    for template in [
        SummaryTemplate::General,
        SummaryTemplate::Standup,
        SummaryTemplate::OneOnOne,
        SummaryTemplate::ClientCall,
    ] {
        if let Some(spec) = template.sections().iter().find(|s| s.key == key) {
            return spec.heading.to_string();
        }
    }
    key.replace('_', " ")
}

pub fn build_markdown(
    title: &str,
    transcript: &LiveTranscript,
    insights: Option<&MeetingInsights>,
    names: &SpeakerNames,
) -> String {
    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n"));
    if let Some(i) = insights {
        if i.sections.is_empty() {
            // Nothing summarised, or summarised before templates had shapes of
            // their own. The three fields are all there is.
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
        } else {
            // Whatever the template asked for, in its order. A client call
            // leaves with its Requirements and Risks rather than with the three
            // headings the general template happens to share.
            for section in &i.sections {
                if section.body.trim().is_empty() {
                    continue;
                }
                md.push_str(&format!("## {}\n\n", section_heading(&section.key)));
                md.push_str(section.body.trim());
                md.push_str("\n\n");
            }
        }
    }
    md.push_str("## Transcript\n\n");
    for seg in transcript.segments() {
        md.push_str(&format!(
            "**{}** ({}): {}\n\n",
            names.label(seg.speaker),
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
    names: &SpeakerNames,
) -> Result<String, String> {
    let md = build_markdown(title, transcript, insights, names);
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
    use crate::domain::i18n::Locale;
    use crate::domain::speaker::Speaker;
    use crate::domain::transcript::TranscriptSegment;
    use tempfile::tempdir;

    fn english() -> SpeakerNames {
        SpeakerNames::resolve(None, None, Locale::En)
    }

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
            sections: Vec::new(),
        };
        (t, insights)
    }

    #[test]
    fn markdown_contains_all_sections() {
        let (t, i) = sample();
        let md = build_markdown("Standup", &t, Some(&i), &english());
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
            export_meeting(&path, fmt, "Standup", &t, Some(&i), &english()).unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            assert!(meta.len() > 10, "{name} should have content");
        }
    }

    /// All three formats are written from the same markdown, so the names have
    /// to be in it — a renamed channel that reached the screen and not the file
    /// is the export saying something the app does not.
    #[test]
    fn the_export_carries_the_meetings_own_names() {
        let dir = tempdir().unwrap();
        let (t, i) = sample();
        let names = SpeakerNames::resolve(Some("Ana"), Some("Cliente"), Locale::En);
        let md = build_markdown("Standup", &t, Some(&i), &names);
        assert!(md.contains("**Ana** ([00:00]): Hello team"), "{md}");
        assert!(md.contains("**Cliente** ([00:01]): Hi there"), "{md}");
        assert!(!md.contains("**Me**"), "{md}");

        let path = dir.path().join("out.md");
        export_meeting(
            &path,
            ExportFormat::Markdown,
            "Standup",
            &t,
            Some(&i),
            &names,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("**Cliente**"));
    }

    #[test]
    fn format_from_ext() {
        assert_eq!(ExportFormat::from_ext("md"), Some(ExportFormat::Markdown));
        assert_eq!(ExportFormat::from_ext("PDF"), Some(ExportFormat::Pdf));
        assert_eq!(ExportFormat::from_ext("docx"), Some(ExportFormat::Docx));
        assert_eq!(ExportFormat::from_ext("txt"), None);
    }

    #[test]
    fn every_format_suggests_an_extension_it_accepts_back() {
        for fmt in [
            ExportFormat::Markdown,
            ExportFormat::Pdf,
            ExportFormat::Docx,
        ] {
            assert_eq!(ExportFormat::from_ext(fmt.extension()), Some(fmt));
        }
    }

    #[test]
    fn a_colon_in_the_title_does_not_reach_the_filesystem() {
        assert_eq!(
            safe_file_stem("Client call: Q3 budget"),
            "Client call - Q3 budget"
        );
        assert_eq!(safe_file_stem("Roadmap 2026/2027"), "Roadmap 2026 - 2027");
        for title in ["a<b", "a>b", "a\"b", "a/b", "a\\b", "a|b", "a?b", "a*b"] {
            let stem = safe_file_stem(title);
            assert!(
                !stem.chars().any(|c| RESERVED.contains(&c)),
                "{title} produced {stem}"
            );
        }
    }

    #[test]
    fn a_control_character_never_reaches_the_name() {
        let stem = safe_file_stem("Sprint\u{7}review\u{1b}");
        assert_eq!(stem, "Sprintreview");
        assert!(!stem.chars().any(char::is_control));
    }

    #[test]
    fn a_trailing_dot_is_stripped_before_the_dialog_sees_it() {
        // Windows strips these silently, so the dialog and the file on disk end
        // up disagreeing about the name unless we strip them first.
        assert_eq!(safe_file_stem("Retro."), "Retro");
        assert_eq!(safe_file_stem("Retro. "), "Retro");
        assert_eq!(safe_file_stem("..."), "meeting");
        assert_eq!(safe_file_stem("   "), "meeting");
        assert_eq!(safe_file_stem(""), "meeting");
    }

    #[test]
    fn a_device_name_is_not_a_filename() {
        for title in ["CON", "con", "NUL.md", "aux", "COM1", "lpt9"] {
            assert_eq!(safe_file_stem(title), "meeting", "{title}");
        }
        // Only the exact names are reserved.
        assert_eq!(safe_file_stem("CONTRACT review"), "CONTRACT review");
        assert_eq!(safe_file_stem("COM0"), "COM0");
        assert_eq!(safe_file_stem("COM10"), "COM10");
    }

    #[test]
    fn an_over_long_title_is_cut_to_one_path_component() {
        let stem = safe_file_stem(&"ação ".repeat(200));
        assert!(stem.chars().map(char::len_utf16).sum::<usize>() <= MAX_STEM_UNITS);
        assert!(!stem.ends_with(' '));
        assert!(stem.starts_with("ação"));

        // Counting chars instead of UTF-16 units would let this one through at
        // twice the length the filesystem allows.
        let emoji = safe_file_stem(&"\u{1f389}".repeat(200));
        assert_eq!(emoji.chars().count(), MAX_STEM_UNITS / 2);
    }
}
