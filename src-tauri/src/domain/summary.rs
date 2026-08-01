use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SummaryTemplate {
    #[default]
    General,
    Standup,
    OneOnOne,
    ClientCall,
}

impl SummaryTemplate {
    pub fn id(self) -> &'static str {
        match self {
            SummaryTemplate::General => "general",
            SummaryTemplate::Standup => "standup",
            SummaryTemplate::OneOnOne => "one_on_one",
            SummaryTemplate::ClientCall => "client_call",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "standup" => SummaryTemplate::Standup,
            "one_on_one" | "1:1" => SummaryTemplate::OneOnOne,
            "client_call" | "client" => SummaryTemplate::ClientCall,
            _ => SummaryTemplate::General,
        }
    }

    pub fn system_prompt(self) -> &'static str {
        match self {
            SummaryTemplate::General => {
                "Summarize the meeting. Produce: (1) Summary, (2) Key points, (3) Action items."
            }
            SummaryTemplate::Standup => {
                "This is a standup. Extract: what was done, what is next, and blockers as action items."
            }
            SummaryTemplate::OneOnOne => {
                "This is a 1:1. Capture goals, feedback, and clear action items with owners."
            }
            SummaryTemplate::ClientCall => {
                "This is a client call. Capture requirements, decisions, risks, and next steps."
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MeetingInsights {
    pub summary: String,
    pub key_points: Vec<String>,
    pub action_items: Vec<String>,
}

impl MeetingInsights {
    pub fn from_model_text(raw: &str) -> Self {
        let mut summary = String::new();
        let mut key_points = Vec::new();
        let mut action_items = Vec::new();
        let mut section = Section::Summary;

        for line in raw.lines() {
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();
            if lower.starts_with("## summary") || lower == "summary:" || lower == "summary" {
                section = Section::Summary;
                continue;
            }
            if lower.starts_with("## key") || lower.starts_with("key points") {
                section = Section::KeyPoints;
                continue;
            }
            if lower.starts_with("## action") || lower.starts_with("action items") {
                section = Section::ActionItems;
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            let bullet = trimmed
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim_start_matches('.')
                .trim();
            match section {
                Section::Summary => {
                    if !summary.is_empty() {
                        summary.push(' ');
                    }
                    summary.push_str(trimmed);
                }
                Section::KeyPoints => key_points.push(bullet.to_string()),
                Section::ActionItems => action_items.push(bullet.to_string()),
            }
        }

        if summary.is_empty() && key_points.is_empty() && action_items.is_empty() {
            summary = raw.trim().to_string();
        }

        Self {
            summary,
            key_points,
            action_items,
        }
    }

    pub fn action_items_text(&self) -> String {
        self.action_items
            .iter()
            .map(|a| format!("- {a}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn key_points_text(&self) -> String {
        self.key_points
            .iter()
            .map(|a| format!("- {a}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Copy)]
enum Section {
    Summary,
    KeyPoints,
    ActionItems,
}

/// Build the user prompt for summarization from a transcript + template.
pub fn build_summary_prompt(template: SummaryTemplate, transcript: &str) -> String {
    format!(
        "{}\n\nRespond in markdown with sections:\n## Summary\n## Key points\n## Action items\n\nTranscript:\n{}",
        template.system_prompt(),
        transcript.trim()
    )
}

/// Offline extractive fallback when no LLM is available.
pub fn extractive_summary(transcript: &str, max_sentences: usize) -> MeetingInsights {
    let sentences: Vec<&str> = transcript
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| s.len() > 12)
        .collect();
    let take = max_sentences
        .min(sentences.len())
        .max(1.min(sentences.len()));
    let summary = sentences
        .iter()
        .take(take)
        .cloned()
        .collect::<Vec<_>>()
        .join(". ");
    let key_points: Vec<String> = sentences
        .iter()
        .take(take.min(5))
        .map(|s| s.to_string())
        .collect();
    let action_items: Vec<String> = transcript
        .lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("todo")
                || l.contains("action")
                || l.contains("will ")
                || l.contains("vamos")
                || l.contains("preciso")
        })
        .map(|l| l.trim().to_string())
        .take(8)
        .collect();
    MeetingInsights {
        summary: if summary.is_empty() {
            "No speech captured yet.".into()
        } else {
            summary
        },
        key_points,
        action_items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_structured_model_output() {
        let raw = r#"
## Summary
We discussed the roadmap.
## Key points
- Shipping v1
- Hiring
## Action items
- Alice drafts RFC
- Bob reviews metrics
"#;
        let i = MeetingInsights::from_model_text(raw);
        assert!(i.summary.contains("roadmap"));
        assert_eq!(i.key_points.len(), 2);
        assert_eq!(i.action_items.len(), 2);
    }

    #[test]
    fn template_prompt_includes_transcript() {
        let p = build_summary_prompt(SummaryTemplate::Standup, "Me: done with API");
        assert!(p.contains("standup") || p.contains("Standup") || p.contains("done with API"));
        assert!(p.contains("done with API"));
    }

    #[test]
    fn extractive_finds_actionish_lines() {
        let t = "Me: hello there everyone.\nOthers: TODO fix login bug.\nMe: we will ship Friday.";
        let i = extractive_summary(t, 3);
        assert!(!i.summary.is_empty());
        assert!(!i.action_items.is_empty());
    }
}
