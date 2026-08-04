use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Build LLM messages for meeting chat: system context + transcript excerpt + history.
pub fn build_chat_context(
    meeting_title: &str,
    transcript: &str,
    summary: Option<&str>,
    history: &[ChatMessage],
    user_question: &str,
    max_transcript_chars: usize,
    notes: &[crate::domain::context::ContextNote],
) -> Vec<ChatMessage> {
    let excerpt = if transcript.chars().count() > max_transcript_chars {
        let truncated: String = transcript.chars().take(max_transcript_chars).collect();
        format!("{truncated}…\n[transcript truncated]")
    } else {
        transcript.to_string()
    };

    let mut system = format!(
        "You are Vesper, a private meeting assistant. Answer only from the meeting context.\nMeeting: {meeting_title}\n"
    );
    if let Some(s) = summary {
        if !s.trim().is_empty() {
            system.push_str(&format!("Summary:\n{s}\n"));
        }
    }
    system.push_str(&format!("Transcript:\n{excerpt}"));
    // After the transcript, like the summary prompt: read the meeting first,
    // then the corrections, and they read as corrections.
    system.push_str(&crate::domain::context::notes_block(notes));

    let mut msgs = vec![ChatMessage {
        role: "system".into(),
        content: system,
    }];
    for h in history {
        if h.role == "user" || h.role == "assistant" {
            msgs.push(h.clone());
        }
    }
    msgs.push(ChatMessage {
        role: "user".into(),
        content: user_question.trim().to_string(),
    });
    msgs
}

/// Offline answer: keyword-hit snippet from transcript (no network).
pub fn offline_answer(transcript: &str, question: &str) -> String {
    let q = question.to_ascii_lowercase();
    let tokens: Vec<&str> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .collect();
    if tokens.is_empty() {
        return "Ask a more specific question about this meeting.".into();
    }
    let mut hits: Vec<&str> = transcript
        .lines()
        .filter(|line| {
            let l = line.to_ascii_lowercase();
            tokens.iter().any(|t| l.contains(t))
        })
        .take(5)
        .collect();
    if hits.is_empty() {
        return "I could not find that in the transcript. Try different keywords.".into();
    }
    hits.insert(0, "Based on the transcript:");
    hits.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_includes_transcript_and_question() {
        let msgs = build_chat_context(
            "Weekly sync",
            "Me: ship the API\nOthers: ok",
            Some("Shipped API plan"),
            &[],
            "What did we decide?",
            10_000,
            &[],
        );
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("Weekly sync"));
        assert!(msgs[0].content.contains("ship the API"));
        assert_eq!(msgs.last().unwrap().role, "user");
        assert!(msgs.last().unwrap().content.contains("decide"));
    }

    #[test]
    fn truncates_long_transcript() {
        let long = "x".repeat(500);
        let msgs = build_chat_context("T", &long, None, &[], "q", 50, &[]);
        assert!(msgs[0].content.contains("truncated"));
    }

    #[test]
    fn offline_answer_hits_keywords() {
        let t = "Me: budget is twelve thousand\nOthers: approved";
        let a = offline_answer(t, "What about budget?");
        assert!(a.to_ascii_lowercase().contains("budget"));
    }
}
