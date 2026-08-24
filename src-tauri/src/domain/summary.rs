use crate::domain::i18n::Locale;
use serde::{Deserialize, Serialize};

/// The heading every template ends with, named once because the prompt has to
/// point the citation instruction at it and a second spelling would point it at
/// a section that does not exist.
const ACTIONS_HEADING: &str = "Action items";

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

    /// What this kind of meeting is made of, in the order it is read.
    ///
    /// The sections are data, not three fields. A standup and a client call
    /// were being asked for Summary / Key points / Action items alike, because
    /// the prompt named those three literally and the parser knew no others —
    /// so picking a template changed the instruction and never the shape of the
    /// answer.
    ///
    /// Every template keeps `summary` first and `action_items` last. The first
    /// is what a person reads when they open the meeting; the last is the one
    /// section that is not text at all — it feeds the action-item list, which
    /// carries owners, deadlines and a done flag of its own.
    pub fn sections(self) -> &'static [SectionSpec] {
        // Named consts, because a slice built inside the `match` is a temporary
        // and cannot be returned as `'static`.
        const SUMMARY: SectionSpec = SectionSpec::prose("summary", "Summary");
        const ACTIONS: SectionSpec = SectionSpec::list("action_items", ACTIONS_HEADING);
        const GENERAL: &[SectionSpec] = &[
            SUMMARY,
            SectionSpec::list("key_points", "Key points"),
            ACTIONS,
        ];
        const STANDUP: &[SectionSpec] = &[
            SUMMARY,
            SectionSpec::list("done", "Done since last time"),
            SectionSpec::list("next", "Next"),
            ACTIONS,
        ];
        const ONE_ON_ONE: &[SectionSpec] = &[
            SUMMARY,
            SectionSpec::list("goals", "Goals"),
            SectionSpec::list("feedback", "Feedback"),
            ACTIONS,
        ];
        const CLIENT_CALL: &[SectionSpec] = &[
            SUMMARY,
            SectionSpec::list("requirements", "Requirements"),
            SectionSpec::list("decisions", "Decisions"),
            SectionSpec::list("risks", "Risks"),
            ACTIONS,
        ];
        match self {
            SummaryTemplate::General => GENERAL,
            SummaryTemplate::Standup => STANDUP,
            SummaryTemplate::OneOnOne => ONE_ON_ONE,
            SummaryTemplate::ClientCall => CLIENT_CALL,
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

/// One section of a summary, declared rather than hardcoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionSpec {
    /// Stable id. Stored with the section and used by the screen to title it
    /// from the catalog, so the heading the model wrote is never shown and
    /// never has to be translated.
    pub key: &'static str,
    /// The heading the model is told to write, in English. English because the
    /// prompt already pins the headings that way and lets the prose follow the
    /// user's language — a model asked for `## Pontos principais` returns three
    /// spellings of it across four runs.
    pub heading: &'static str,
    /// Prose joins its lines into a paragraph; a list keeps them apart and
    /// strips the bullet.
    pub prose: bool,
}

impl SectionSpec {
    const fn prose(key: &'static str, heading: &'static str) -> Self {
        Self {
            key,
            heading,
            prose: true,
        }
    }
    const fn list(key: &'static str, heading: &'static str) -> Self {
        Self {
            key,
            heading,
            prose: false,
        }
    }
}

/// `- ` lines, which is how a list section's body is stored.
fn bullets(items: &[String]) -> String {
    items
        .iter()
        .map(|i| format!("- {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A section as the model answered it, ready to store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SummarySection {
    pub key: String,
    /// Markdown. Prose as written; a list as `- ` lines, which is what the
    /// renderer and the exporter both already read.
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MeetingInsights {
    pub summary: String,
    pub key_points: Vec<String>,
    pub action_items: Vec<String>,
    /// Every section the template asked for, in order.
    ///
    /// The three fields above are the general template's own sections, kept
    /// because the search index, the export, the chat context and the
    /// action-item merge all read them by name. They are filled from here when
    /// the keys match rather than parsed twice.
    #[serde(default)]
    pub sections: Vec<SummarySection>,
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

/// Which section a line opens, if it opens one.
///
/// The prompt asks for the three headings in English and most models comply,
/// but the better a model writes the target language the likelier it is to
/// translate them too: Gemma 3 4B produced the best Portuguese prose in the
/// bench and titled it `## Resumo`. Matching English only, the whole answer
/// fell into the summary and the other two sections came back empty — the best
/// output scored worst. So the headings are read in both shipped languages.
///
/// Bold (`**Resumo**`) and trailing colons appear about as often as plain ones,
/// and cost a line each to accept.
/// A heading line reduced to a comparable label, and whether it was hashed.
///
/// `None` when the line is not a heading at all.
fn heading_label(line: &str) -> (String, bool) {
    let hashed = line.starts_with('#');
    // Looped, because the decoration nests in either order: `**Key points:**`
    // puts the colon inside the bold and `**Key points**:` puts it outside.
    // One pass in a fixed order strips whichever came first and then stalls on
    // the other, leaving a label that matches nothing — and a heading that
    // matches nothing silently pours its section into the previous one.
    let mut label = line.trim_start_matches('#').trim().to_string();
    loop {
        let stripped = label
            .trim()
            .trim_start_matches("**")
            .trim_end_matches("**")
            .trim_end_matches(':')
            .trim();
        if stripped.len() == label.len() {
            break;
        }
        label = stripped.to_string();
    }
    (label.to_lowercase(), hashed)
}

/// The Portuguese a model reaches for when it translates a heading it was told
/// to keep in English.
///
/// Every section has them, not just the shipped three. The better a model
/// writes the target language the likelier it is to translate the headings too
/// — that is why the legacy three needed aliases in the first place — and a
/// `## Riscos` nobody recognises does not come back empty, it pours its lines
/// into whichever section came before it.
///
/// `próximos passos` appears under `next` as well as `action_items`. Order
/// decides: `next` sits earlier in the standup's list, so a standup files it as
/// what happens next, and every other template still reads it as a task. That
/// is the right answer for both — a standup's next steps are its own section.
fn aliases(key: &str) -> &'static [&'static str] {
    match key {
        "summary" => &["resumo"],
        "key_points" => &["pontos-chave", "pontos chave", "pontos principais"],
        "action_items" => &["itens de ação", "ações", "próximos passos"],
        "done" => &["feito", "feito desde a última vez", "concluído"],
        "next" => &["próximos", "próximos passos", "a seguir"],
        "goals" => &["objetivos", "metas"],
        "feedback" => &["retorno", "devolutiva"],
        "requirements" => &["requisitos"],
        "decisions" => &["decisões"],
        "risks" => &["riscos"],
        _ => &[],
    }
}

/// Which of `specs` a line opens, if it opens one.
fn heading_index(specs: &[SectionSpec], line: &str) -> Option<usize> {
    let (label, hashed) = heading_label(line);
    if label.is_empty() {
        return None;
    }
    // A bare word only opens a section when it is the whole line. Without that,
    // a summary whose first sentence starts "Resumo da reunião…" would be eaten
    // as a heading.
    let opens = |name: &str| {
        if hashed {
            label.starts_with(name)
        } else {
            label == name
        }
    };
    specs.iter().position(|spec| {
        let heading = spec.heading.to_lowercase();
        // The singular too: a model with one item to report writes `## Action
        // item` about as often as not.
        let singular = heading.strip_suffix('s').unwrap_or(&heading).to_string();
        opens(&heading) || opens(&singular) || aliases(spec.key).iter().any(|a| opens(a))
    })
}

/// Strip a bullet or a numbered prefix from a list line.
fn strip_bullet(line: &str) -> &str {
    line.trim_start_matches('-')
        .trim_start_matches('*')
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches('.')
        .trim()
}

impl MeetingInsights {
    /// Parse an answer against the sections its template asked for.
    ///
    /// The heading is matched on the spec's own text, so a template that asks
    /// for Risks gets a Risks section without anything here knowing what a risk
    /// is. The legacy three keep their Portuguese aliases — a model writing
    /// `## Resumo` was collapsing the whole answer into one field, and that is
    /// the shipped catalogue's behaviour, not a template's.
    pub fn from_model_text_for(template: SummaryTemplate, raw: &str) -> Self {
        let specs = template.sections();
        let mut bodies: Vec<Vec<String>> = vec![Vec::new(); specs.len()];
        // Anything before the first heading belongs to the opening section,
        // which is the summary in every template: a model that answers with a
        // paragraph and then starts using headings is common, and dropping that
        // paragraph loses the only part most people read.
        let mut at = 0usize;
        let mut saw_heading = false;

        for line in raw.lines() {
            let trimmed = line.trim();
            if let Some(i) = heading_index(specs, trimmed) {
                at = i;
                saw_heading = true;
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            if specs[at].prose {
                bodies[at].push(trimmed.to_string());
            } else {
                bodies[at].push(strip_bullet(trimmed).to_string());
            }
        }

        let sections: Vec<SummarySection> = specs
            .iter()
            .zip(bodies.iter())
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(spec, lines)| SummarySection {
                key: spec.key.to_string(),
                body: if spec.prose {
                    lines.join(" ")
                } else {
                    lines
                        .iter()
                        .map(|l| format!("- {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            })
            .collect();

        let list_of = |key: &str| -> Vec<String> {
            specs
                .iter()
                .position(|s| s.key == key)
                .map(|i| bodies[i].clone())
                .unwrap_or_default()
        };
        let mut summary = specs
            .iter()
            .position(|s| s.key == "summary")
            .map(|i| bodies[i].join(" "))
            .unwrap_or_default();
        // No heading anywhere and nothing recognised: the answer is the summary.
        // Better than an empty screen beside a model that did reply.
        if !saw_heading && summary.is_empty() && sections.is_empty() {
            summary = raw.trim().to_string();
        }

        Self {
            summary,
            key_points: list_of("key_points"),
            action_items: list_of("action_items"),
            sections,
        }
    }

    /// The general template's three, for callers that have no template to hand
    /// — the extractive fallback and the tests that predate the others.
    pub fn from_model_text(raw: &str) -> Self {
        Self::from_model_text_for(SummaryTemplate::General, raw)
    }

    /// Push the three legacy fields back into the sections that mirror them.
    ///
    /// The sections carry a second copy of the same text. Refinement improves
    /// one field by name and knows nothing about the list, so without this the
    /// screen and the export would both keep showing the body from before the
    /// improvement — the copy that is stored wins over the copy that changed.
    ///
    /// Only the three that have a field. A client call's Risks are not mirrored
    /// anywhere and are left exactly as they are.
    pub fn sync_sections(&mut self) {
        if self.sections.is_empty() {
            return;
        }
        for section in &mut self.sections {
            match section.key.as_str() {
                "summary" => section.body = self.summary.clone(),
                "key_points" => section.body = bullets(&self.key_points),
                "action_items" => section.body = bullets(&self.action_items),
                _ => {}
            }
        }
        // A field with no section is not nothing to say. The model can answer a
        // client call without an Action items heading, and the user then adds
        // three tasks in the panel — which writes the rows and the column and
        // never this list, so an export built from the list alone would drop
        // them. The same holds for a Key points improved into a section the
        // model never wrote.
        //
        // Positions are the ones every template declares: summary opens and
        // action items close.
        let missing = |list: &Vec<SummarySection>, key: &str| !list.iter().any(|s| s.key == key);
        if !self.summary.is_empty() && missing(&self.sections, "summary") {
            self.sections.insert(
                0,
                SummarySection {
                    key: "summary".into(),
                    body: self.summary.clone(),
                },
            );
        }
        if !self.key_points.is_empty() && missing(&self.sections, "key_points") {
            let at = self
                .sections
                .iter()
                .position(|s| s.key == "action_items")
                .unwrap_or(self.sections.len());
            self.sections.insert(
                at,
                SummarySection {
                    key: "key_points".into(),
                    body: bullets(&self.key_points),
                },
            );
        }
        if !self.action_items.is_empty() && missing(&self.sections, "action_items") {
            self.sections.push(SummarySection {
                key: "action_items".into(),
                body: bullets(&self.action_items),
            });
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

/// The language the model is told to write in, named in English.
///
/// It lives here rather than on `Locale` because it is prompt text, not UI
/// copy: an English system prompt asking the model to "responda em Inglês" is
/// worse than one asking it to "answer in English", and a translated entry in
/// the i18n dictionaries would be a key no screen ever renders.
/// The same instruction, written in the language it asks for.
///
/// A prompt is entirely English apart from the transcript, and a small model
/// answers in the language it was addressed in far more reliably than in the
/// one it was told about. One sentence in the target language, placed last,
/// costs nothing and is the difference between a request and a hint.
pub fn write_in_language(locale: Locale) -> &'static str {
    match locale {
        Locale::En => "Write everything in English.",
        Locale::PtBr => "Escreva tudo em português do Brasil.",
    }
}

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
    build_summary_prompt_with(template, transcript, locale, &[], None)
}

/// The same prompt, plus whatever the participant typed while it was happening.
///
/// The notes go after the transcript rather than before it. A model that reads
/// an instruction first and the evidence second tends to answer the
/// instruction; reading the meeting and then the corrections treats them as
/// corrections, which is what they are.
pub fn build_summary_prompt_with(
    template: SummaryTemplate,
    transcript: &str,
    locale: Locale,
    notes: &[crate::domain::context::ContextNote],
    brief: Option<&str>,
) -> String {
    // The headings come from the template rather than from this string. Naming
    // them here is what made every template answer in the same three sections
    // however differently it was introduced.
    let specs = template.sections();
    let headings = specs
        .iter()
        .map(|s| format!("## {}", s.heading))
        .collect::<Vec<_>>()
        .join("\n");
    let names = specs
        .iter()
        .map(|s| format!("## {}", s.heading))
        .collect::<Vec<_>>()
        .join(", ");
    // Every template ends in action items, and every action item is asked to
    // name the moment it was decided. The transcript arrives stamped, so this is
    // a copy rather than a judgement — and an item the model invented has no
    // moment to copy, which is the point: a citation that does not play back is
    // visible, where a plausible sentence is not.
    let cite = format!(
        "\n\nEnd every line under `## {}` with the moment it was decided, in \
         square brackets, copied from the timestamp of the transcript line it \
         came from: `[mm:ss]`. Put it last, after any owner or deadline. Leave \
         it off only when the transcript does not show when it came up — never \
         guess one.",
        ACTIONS_HEADING
    );
    format!(
        "{}\n\nRespond in markdown with sections:\n{}\n\nWrite all prose in {}.\nKeep the headings exactly as written, in English: {}.{}{}\n\nTranscript:\n{}{}\n\n{}",
        template.system_prompt(),
        headings,
        language_name(locale),
        names,
        // Beside the other instructions about the shape of the answer, since
        // that is what it is about.
        cite,
        // Before the transcript, because it is what the transcript is to be
        // read against. After it, a model treats it as one more thing that
        // was said near the end.
        crate::domain::context::brief_block(brief),
        transcript.trim(),
        crate::domain::context::notes_block(notes),
        // Last, and in the language it asks for. Everything above is English
        // and so is the instruction above, which leaves a small model reading a
        // wall of English and answering in kind — the headings are supposed to
        // be the only English in the answer.
        write_in_language(locale)
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
        // No sections. This runs when there is no model at all, and the three
        // fields are exactly what it can produce — leaving the list empty is
        // what puts the screen on its fallback, which renders those three.
        sections: Vec::new(),
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
    fn notes_reach_the_prompt_after_the_transcript() {
        let notes = vec![crate::domain::context::ContextNote {
            id: 1,
            text: "Cliente = Acme".into(),
            at_ms: Some(5_000),
            created_at: "2026-08-04T00:00:00Z".into(),
        }];
        let p = build_summary_prompt_with(
            SummaryTemplate::General,
            "falamos do contrato",
            Locale::PtBr,
            &notes,
            None,
        );
        assert!(p.contains("Cliente = Acme"), "{p}");
        assert!(
            p.find("falamos do contrato") < p.find("Cliente = Acme"),
            "notes must follow the transcript"
        );
    }

    /// The brief is standing context, the notes are corrections made during the
    /// call, and the order they reach the model in is the whole difference: read
    /// first, the brief frames the transcript; read last, it is one more thing
    /// somebody said near the end.
    #[test]
    fn the_brief_comes_before_the_transcript_and_the_notes_after() {
        let notes = vec![crate::domain::context::ContextNote {
            id: 1,
            text: "Cliente = Acme".into(),
            at_ms: Some(5_000),
            created_at: "2026-08-04T00:00:00Z".into(),
        }];
        let p = build_summary_prompt_with(
            SummaryTemplate::ClientCall,
            "falamos do contrato",
            Locale::PtBr,
            &notes,
            Some("Revisão trimestral da Acme"),
        );
        let brief = p.find("Revisão trimestral da Acme").expect("brief");
        let transcript = p.find("falamos do contrato").expect("transcript");
        let note = p.find("Cliente = Acme").expect("note");
        assert!(brief < transcript, "the brief frames the transcript: {p}");
        assert!(transcript < note, "the notes correct it: {p}");
    }

    /// A meeting with no notes must read exactly as it did before, or every
    /// existing summary silently changes shape.
    #[test]
    fn no_notes_leaves_the_prompt_untouched() {
        let plain = build_summary_prompt(SummaryTemplate::General, "oi", Locale::En);
        let with = build_summary_prompt_with(SummaryTemplate::General, "oi", Locale::En, &[], None);
        assert_eq!(plain, with);
    }

    /// The instruction has to name the section it applies to, and it has to
    /// name it the way the prompt asked for it a few lines above. Two spellings
    /// point the model at a heading that does not exist.
    #[test]
    fn every_template_points_the_citation_at_its_own_action_heading() {
        for template in [
            SummaryTemplate::General,
            SummaryTemplate::Standup,
            SummaryTemplate::OneOnOne,
            SummaryTemplate::ClientCall,
        ] {
            let p = build_summary_prompt(template, "Me: ship it", Locale::En);
            let heading = template
                .sections()
                .last()
                .expect("every template ends in action items")
                .heading;
            assert!(
                p.contains(&format!("End every line under `## {heading}`")),
                "{template:?}: {p}"
            );
            assert!(p.contains("[mm:ss]"), "{template:?}");
            assert!(p.contains("never guess one"), "{template:?}");
        }
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
            assert!(p.contains("Keep the headings exactly as written, in English"));
        }
    }

    /// The prompt names the template's own sections, not three fixed ones.
    /// Picking a template used to change the instruction and never the shape of
    /// the answer.
    #[test]
    fn each_template_asks_for_its_own_sections() {
        let p = build_summary_prompt(SummaryTemplate::ClientCall, "Me: hi", Locale::En);
        for heading in ["## Summary", "## Requirements", "## Decisions", "## Risks"] {
            assert!(p.contains(heading), "missing {heading}");
        }
        assert!(
            !p.contains("## Key points"),
            "a client call is not a general one"
        );
    }

    #[test]
    fn a_template_parses_into_its_own_sections() {
        let raw = "## Summary\nThe call went well.\n## Requirements\n- SSO\n- Audit log\n## Risks\n- Timeline\n## Action items\n- Send the quote";
        let out = MeetingInsights::from_model_text_for(SummaryTemplate::ClientCall, raw);
        let keys: Vec<&str> = out.sections.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["summary", "requirements", "risks", "action_items"]);
        assert_eq!(out.summary, "The call went well.");
        // Decisions was never written, so it is absent rather than empty: a card
        // holding one em-dash is worse than no card.
        assert!(out.sections.iter().all(|s| s.key != "decisions"));
        // And the legacy field the action list feeds is still filled.
        assert_eq!(out.action_items, ["Send the quote"]);
    }

    /// A model that writes good Portuguese translates the headings it was told
    /// to keep in English. Unrecognised, `## Riscos` does not come back empty —
    /// its lines pour into whatever section came before it.
    #[test]
    fn a_translated_heading_still_finds_its_section() {
        let raw = "## Resumo\nA chamada foi boa.\n## Requisitos\n- SSO\n## Riscos\n- Prazo";
        let out = MeetingInsights::from_model_text_for(SummaryTemplate::ClientCall, raw);
        let keys: Vec<&str> = out.sections.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["summary", "requirements", "risks"]);
        assert_eq!(out.summary, "A chamada foi boa.");
    }

    /// `Próximos passos` is a standup's own section and a task list everywhere
    /// else. Order in the template decides, and both readings are right.
    #[test]
    fn next_steps_belong_to_the_standup_and_to_the_tasks_elsewhere() {
        let raw = "## Resumo\nOk.\n## Próximos passos\n- Revisar o PR";
        let standup = MeetingInsights::from_model_text_for(SummaryTemplate::Standup, raw);
        assert!(standup.sections.iter().any(|s| s.key == "next"));
        assert!(standup.action_items.is_empty(), "not a task in a standup");
        let general = MeetingInsights::from_model_text_for(SummaryTemplate::General, raw);
        assert_eq!(general.action_items, ["Revisar o PR"]);
    }

    /// Improving one field has to reach the copy of it that is stored, or the
    /// screen keeps showing the body from before the improvement.
    #[test]
    fn refining_a_field_reaches_its_section() {
        let raw = "## Summary\nWe met.\n## Requirements\n- SSO\n## Action items\n- Old";
        let mut out = MeetingInsights::from_model_text_for(SummaryTemplate::ClientCall, raw);
        out.action_items = vec!["New".into()];
        out.sync_sections();
        let actions = out
            .sections
            .iter()
            .find(|s| s.key == "action_items")
            .expect("the section survived");
        assert_eq!(actions.body, "- New");
        // And a section with no field of its own is left exactly as it was.
        let reqs = out
            .sections
            .iter()
            .find(|s| s.key == "requirements")
            .unwrap();
        assert_eq!(reqs.body, "- SSO");
    }

    /// The model can answer without an Action items heading, and the user then
    /// adds tasks in the panel. Those write the rows and the column, never this
    /// list — so a sync that only replaced what was already there would export
    /// a meeting with the user's tasks missing.
    #[test]
    fn a_field_with_no_section_gains_one() {
        let raw = "## Summary\nWe met.\n## Requirements\n- SSO";
        let mut out = MeetingInsights::from_model_text_for(SummaryTemplate::ClientCall, raw);
        assert!(out.sections.iter().all(|s| s.key != "action_items"));
        out.action_items = vec!["Send the quote".into()];
        out.sync_sections();
        let keys: Vec<&str> = out.sections.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["summary", "requirements", "action_items"]);
        assert_eq!(out.sections.last().unwrap().body, "- Send the quote");
    }

    /// And a meeting that predates sections keeps none: an empty list is what
    /// puts the screen and the export on the legacy columns.
    #[test]
    fn a_meeting_without_sections_does_not_grow_them() {
        let mut out = MeetingInsights {
            summary: "Old".into(),
            action_items: vec!["Task".into()],
            ..Default::default()
        };
        out.sync_sections();
        assert!(out.sections.is_empty());
    }

    /// The general template still answers exactly as it did, because every
    /// meeting already summarised was summarised by it.
    #[test]
    fn the_general_template_is_unchanged() {
        let raw = "## Summary\nWe met.\n## Key points\n- One\n## Action items\n- Two";
        let out = MeetingInsights::from_model_text(raw);
        assert_eq!(out.summary, "We met.");
        assert_eq!(out.key_points, ["One"]);
        assert_eq!(out.action_items, ["Two"]);
        assert_eq!(out.sections.len(), 3);
    }

    /// Bold and a colon nest in either order, and both orders occur.
    #[test]
    fn a_heading_survives_its_decoration() {
        for heading in [
            "**Pontos-chave:**",
            "**Pontos-chave**:",
            "## **Key points**",
            "Key points:",
        ] {
            let raw = format!(
                "## Resumo
x
{heading}
- um ponto"
            );
            let i = MeetingInsights::from_model_text(&raw);
            assert_eq!(i.key_points, vec!["um ponto"], "failed on {heading}");
        }
    }

    /// Gemma 3 4B's real shape: it translates the headings it was told to keep.
    #[test]
    fn portuguese_headings_still_find_their_sections() {
        let raw = "## Resumo
Fechamos o escopo.

**Pontos-chave**
- Instalador trava

## Ações:
- Assinar o build";
        let i = MeetingInsights::from_model_text(raw);
        assert_eq!(i.summary, "Fechamos o escopo.");
        assert_eq!(i.key_points, vec!["Instalador trava"]);
        assert_eq!(i.action_items, vec!["Assinar o build"]);
    }

    /// Prose that merely opens with the word must not be mistaken for a heading.
    #[test]
    fn a_sentence_starting_with_resumo_is_not_a_heading() {
        let i = MeetingInsights::from_model_text(
            "## Resumo
Resumo da reunião: escopo fechado.",
        );
        assert_eq!(i.summary, "Resumo da reunião: escopo fechado.");
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
