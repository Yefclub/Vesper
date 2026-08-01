use serde::{Deserialize, Serialize};

/// One search result, as the UI renders it.
///
/// The ranking that used to live here — a hand-written term scorer fed with every
/// meeting loaded into memory — was replaced by the SQLite FTS5 index, which does
/// the same job without materialising every transcript on each keystroke. The
/// query now lives in `Database::search`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub meeting_id: String,
    pub title: String,
    pub snippet: String,
    /// Higher is more relevant. FTS5 reports bm25 the other way round, so the
    /// database layer negates it before it reaches here.
    pub score: f64,
}
