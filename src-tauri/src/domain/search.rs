use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub meeting_id: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub meeting_id: String,
    pub title: String,
    pub body: String,
}

/// Simple ranked full-text search over local meeting documents (no external index).
pub fn search_meetings(docs: &[SearchDocument], query: &str, limit: usize) -> Vec<SearchHit> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let terms: Vec<&str> = q
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<SearchHit> = docs
        .iter()
        .filter_map(|d| {
            let title_l = d.title.to_ascii_lowercase();
            let body_l = d.body.to_ascii_lowercase();
            let mut score = 0.0f32;
            for t in &terms {
                if title_l.contains(t) {
                    score += 3.0;
                }
                if body_l.contains(t) {
                    score += 1.0;
                }
            }
            if score <= 0.0 {
                return None;
            }
            let snippet = make_snippet(&d.body, terms[0], 120);
            Some(SearchHit {
                meeting_id: d.meeting_id.clone(),
                title: d.title.clone(),
                snippet,
                score,
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.cmp(&b.title))
    });
    hits.truncate(limit.max(1));
    hits
}

fn make_snippet(body: &str, term: &str, width: usize) -> String {
    let lower = body.to_ascii_lowercase();
    let term_l = term.to_ascii_lowercase();
    if let Some(pos) = lower.find(&term_l) {
        let start = pos.saturating_sub(width / 3);
        let end = (pos + term.len() + width * 2 / 3).min(body.len());
        // byte slicing is safe here because we search on ascii-lowercased body indices
        // only when term is ascii; fallback to char boundary adjust:
        let start = floor_char_boundary(body, start);
        let end = ceil_char_boundary(body, end);
        let mut s = body[start..end].to_string();
        if start > 0 {
            s = format!("…{s}");
        }
        if end < body.len() {
            s = format!("{s}…");
        }
        s
    } else {
        body.chars().take(width).collect()
    }
}

fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut j = i;
    while j > 0 && !s.is_char_boundary(j) {
        j -= 1;
    }
    j
}

fn ceil_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut j = i;
    while j < s.len() && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs() -> Vec<SearchDocument> {
        vec![
            SearchDocument {
                meeting_id: "1".into(),
                title: "Budget review".into(),
                body: "Me: Q3 budget needs cut. Others: approve marketing.".into(),
            },
            SearchDocument {
                meeting_id: "2".into(),
                title: "Engineering standup".into(),
                body: "Me: shipped auth. Others: flaky tests.".into(),
            },
        ]
    }

    #[test]
    fn finds_by_title_and_body() {
        let hits = search_meetings(&docs(), "budget", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, "1");
        assert!(hits[0].snippet.to_ascii_lowercase().contains("budget"));
    }

    #[test]
    fn ranks_title_higher() {
        let hits = search_meetings(&docs(), "standup", 10);
        assert_eq!(hits[0].meeting_id, "2");
        assert!(hits[0].score >= 3.0);
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(search_meetings(&docs(), "  ", 5).is_empty());
    }
}
