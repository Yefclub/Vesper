use crate::domain::i18n::Locale;
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

/// Split a reasoning model's thinking from its answer.
///
/// Qwen3 and its kind write their working inside `<think>…</think>` before
/// answering. Fed straight to the parser that reads headings, that working
/// becomes the summary — the user gets a paragraph of the model talking to
/// itself where the meeting should be.
///
/// Returns `(thinking, answer)`. A model that does not think returns the whole
/// text as the answer, which is every model in the catalog today.
pub fn split_thinking(raw: &str) -> (Option<String>, String) {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";

    let Some(start) = raw.find(OPEN) else {
        return (None, raw.to_string());
    };
    match raw[start..].find(CLOSE) {
        Some(offset) => {
            let inner = &raw[start + OPEN.len()..start + offset];
            let mut answer = String::from(&raw[..start]);
            answer.push_str(&raw[start + offset + CLOSE.len()..]);
            let thinking = inner.trim().to_string();
            (
                (!thinking.is_empty()).then_some(thinking),
                answer.trim().to_string(),
            )
        }
        // Thinking that ran out of budget before closing. Everything after the
        // tag is working, not answer — returning it as the summary would be
        // worse than returning nothing.
        None => {
            let thinking = raw[start + OPEN.len()..].trim().to_string();
            (
                (!thinking.is_empty()).then_some(thinking),
                raw[..start].trim().to_string(),
            )
        }
    }
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

/// The language the model is told to write in, named in English.
///
/// It lives here rather than on `Locale` because it is prompt text, not UI
/// copy: an English system prompt asking the model to "responda em Inglês" is
/// worse than one asking it to "answer in English", and a translated entry in
/// the i18n dictionaries would be a key no screen ever renders.
pub fn language_name(locale: Locale) -> &'static str {
    match locale {
        Locale::En => "English",
        Locale::PtBr => "Brazilian Portuguese",
    }
}

/// Build the user prompt for summarization from a transcript + template.
///
/// The prose follows the app's locale; the three markdown headings do not.
/// They are a wire protocol: `MeetingInsights::from_model_text` switches
/// sections on `## summary` / `## key` / `## action`, so a model that answers
/// with `## Resumo` / `## Pontos principais` / `## Ações` collapses the whole
/// answer into `summary` and persists two empty vectors. The headings are never
/// shown — the cards title themselves from the catalog.
pub fn build_summary_prompt(template: SummaryTemplate, transcript: &str, locale: Locale) -> String {
    format!(
        "{}\n\nRespond in markdown with sections:\n## Summary\n## Key points\n## Action items\n\nWrite all prose in {}.\nKeep the three headings exactly as written, in English: ## Summary, ## Key points, ## Action items.\n\nTranscript:\n{}",
        template.system_prompt(),
        language_name(locale),
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
mod thinking {
    use super::*;

    #[test]
    fn a_model_that_does_not_think_is_untouched() {
        let (thinking, answer) = split_thinking("## Resumo\nDecidimos o escopo.");
        assert!(thinking.is_none());
        assert_eq!(answer, "## Resumo\nDecidimos o escopo.");
    }

    #[test]
    fn thinking_is_lifted_out_of_the_answer() {
        let (thinking, answer) =
            split_thinking("<think>Preciso listar os pontos.</think>\n## Resumo\nEscopo fechado.");
        assert_eq!(thinking.as_deref(), Some("Preciso listar os pontos."));
        assert_eq!(answer, "## Resumo\nEscopo fechado.");
    }

    /// A budget that ran out mid-thought leaves no closing tag. Handing the
    /// remainder over as the summary would show the user the model muttering.
    #[test]
    fn unfinished_thinking_does_not_become_the_summary() {
        let (thinking, answer) = split_thinking("<think>Primeiro eu preciso");
        assert_eq!(thinking.as_deref(), Some("Primeiro eu preciso"));
        assert!(answer.is_empty());
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
        let p = build_summary_prompt(SummaryTemplate::Standup, "Me: done with API", Locale::En);
        assert!(p.contains("standup") || p.contains("Standup") || p.contains("done with API"));
        assert!(p.contains("done with API"));
    }

    #[test]
    fn a_portuguese_locale_asks_for_portuguese_prose() {
        let p = build_summary_prompt(SummaryTemplate::General, "Me: bom dia", Locale::PtBr);
        assert!(p.contains("Write all prose in Brazilian Portuguese."));
    }

    #[test]
    fn the_markdown_headings_stay_english_in_every_locale() {
        for locale in [Locale::En, Locale::PtBr] {
            let p = build_summary_prompt(SummaryTemplate::General, "Me: hi", locale);
            assert!(p.contains("## Summary"));
            assert!(p.contains("## Key points"));
            assert!(p.contains("## Action items"));
            assert!(p.contains("Keep the three headings exactly as written, in English"));
        }
    }

    #[test]
    fn sections_still_parse_when_the_body_is_portuguese() {
        // What the model answers once it is told to write in Portuguese and to
        // keep the headings. Lose the second half of that instruction and every
        // branch below misses, the whole answer lands in `summary`, and two
        // empty vectors get persisted.
        let raw = r#"
## Summary
Discutimos o roadmap do trimestre.
## Key points
- Enviar a v1
- Contratações
## Action items
- Alice redige o RFC
- Bob revisa as métricas
"#;
        let i = MeetingInsights::from_model_text(raw);
        assert!(i.summary.contains("roadmap"));
        assert_eq!(i.key_points.len(), 2);
        assert_eq!(i.action_items.len(), 2);
    }

    #[test]
    fn extractive_finds_actionish_lines() {
        let t = "Me: hello there everyone.\nOthers: TODO fix login bug.\nMe: we will ship Friday.";
        let i = extractive_summary(t, 3);
        assert!(!i.summary.is_empty());
        assert!(!i.action_items.is_empty());
    }
}
