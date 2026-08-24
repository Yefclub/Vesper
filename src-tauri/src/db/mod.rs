use crate::domain::job::{MeetingRecord, MeetingStatus};
use crate::domain::refine::SummaryVersion;
use crate::domain::search::SearchHit;
use crate::domain::settings::AppSettings;
use crate::domain::speaker::{Speaker, SpeakerNames};
use crate::domain::summary::MeetingInsights;
use crate::domain::transcript::{LiveTranscript, TranscriptSegment};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
    data_dir: PathBuf,
}

/// Deletes a meeting's recording, refusing anything outside the app's own
/// recordings directory.
///
/// The containment check runs on canonicalised paths. `Path::starts_with`
/// compares components without resolving them, so a stored path of
/// `<recordings>/../../something` satisfies it and the delete would reach outside
/// — turning a tampered database row into an arbitrary file deletion.
fn delete_recording(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        // Already gone, or never written. Nothing to remove and nothing to report.
        return Ok(());
    }
    let root = crate::paths::recordings_dir();
    let (Ok(root), Ok(resolved)) = (root.canonicalize(), path.canonicalize()) else {
        return Err("could not resolve the recording's location".into());
    };
    if !resolved.starts_with(&root) {
        tracing::warn!("refusing to delete a recording stored outside the app directory");
        return Ok(());
    }
    std::fs::remove_file(&resolved)
        .map_err(|e| format!("the recording could not be deleted, so nothing was removed: {e}"))
}

/// Turns a user's typed query into an FTS5 MATCH expression.
///
/// Raw input cannot go in: `AND`, `*`, `"` and `:` are operators there, so a
/// perfectly ordinary search like `budget: Q3` is a syntax error rather than a
/// search. Each word becomes a quoted phrase, with quotes doubled to escape them.
///
/// The terms are joined with `OR`, not the implicit `AND`, because that is what
/// the search this replaces did: a meeting matched when any term appeared, and
/// the rest only raised its rank. bm25 still sorts documents carrying more of the
/// terms to the top, so the useful part of `AND` survives without the part that
/// makes a two-word search find nothing.
///
/// Each phrase is a prefix query. The search runs as the user types, and the
/// scorer this replaces matched substrings — without the `*`, typing `plan`
/// returns nothing until `planning` is complete, which reads as a broken search.
fn fts_match_expression(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.replace('"', "\"\""))
        .filter(|t| !t.trim().is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(terms.join(" OR "))
}

/// Where the API key is safe to leave when settings are written.
///
/// Exists so the dangerous case cannot be reached by forgetting: the row may only
/// drop the key once something else is holding it. `KeepInRow` is not a
/// preference — it is the legacy plaintext home, kept alive purely so a machine
/// whose keychain refuses to cooperate does not end up with the user's credential
/// stored nowhere at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyHome {
    Keychain,
    KeepInRow,
}

impl Database {
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let db_path = data_dir.join("vesper.db");
        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
        let db = Self {
            conn: Mutex::new(conn),
            data_dir: data_dir.to_path_buf(),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        // SQLite defaults foreign_keys to OFF, per connection. Without this the
        // ON DELETE CASCADE declared below never fires and the constraint is
        // decorative — a deleted meeting would leave its rows behind.
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| e.to_string())?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meetings (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                audio_path TEXT,
                transcript_text TEXT NOT NULL DEFAULT '',
                summary TEXT,
                action_items TEXT,
                key_points TEXT,
                project TEXT
            );
            CREATE TABLE IF NOT EXISTS transcript_segments (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL,
                speaker TEXT NOT NULL,
                text TEXT NOT NULL,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS chat_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );
            -- Every version of a meeting's insights, append-only. A version is
            -- the WHOLE set — summary, key points, action items — not a delta:
            -- the three are read together, so restoring one has to yield a
            -- coherent set without replaying a chain.
            CREATE TABLE IF NOT EXISTS summary_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                origin TEXT NOT NULL,
                created_at TEXT NOT NULL,
                summary TEXT NOT NULL,
                key_points TEXT NOT NULL,
                action_items TEXT NOT NULL,
                UNIQUE(meeting_id, version)
            );
            CREATE INDEX IF NOT EXISTS idx_versions_meeting ON summary_versions(meeting_id);
            -- What the user typed while the meeting was happening. Not
            -- speech and not the model's: the person in the room knows the
            -- spelling of a client's name and which two minutes to ignore, and
            -- neither reaches the transcript on its own.
            --
            -- `at_ms` is the offset from the start of the recording, so a note
            -- keeps its place against the transcript. Null for a note added
            -- after the recording ended, which has no offset to keep.
            CREATE TABLE IF NOT EXISTS context_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id TEXT NOT NULL,
                text TEXT NOT NULL,
                at_ms INTEGER,
                created_at TEXT NOT NULL,
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_context_meeting ON context_notes(meeting_id);
            -- What the meeting decided somebody would do.
            --
            -- The `meetings.action_items` text column stays: it is what export
            -- and the search index read, and it is rewritten from these rows.
            -- Two representations of one truth is a cost, and the alternative
            -- was rewriting both of those readers in the same change.
            CREATE TABLE IF NOT EXISTS action_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id TEXT NOT NULL,
                text TEXT NOT NULL,
                owner TEXT,
                due TEXT,
                status TEXT NOT NULL DEFAULT 'open',
                source TEXT NOT NULL DEFAULT 'ai',
                -- Touched by a person. Never cleared: it is what stops the next
                -- summary from replacing their words with the model's.
                edited INTEGER NOT NULL DEFAULT 0,
                position INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_actions_meeting ON action_items(meeting_id);
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_segments_meeting ON transcript_segments(meeting_id);
            CREATE INDEX IF NOT EXISTS idx_chat_meeting ON chat_messages(meeting_id);
            -- Search index. Without it every keystroke pulled every transcript out
            -- of the database and concatenated it in memory to scan by hand.
            -- remove_diacritics keeps "reuniao" matching "reunião".
            -- prefix='2 3' because the search runs as the user types: without
            -- these, the first couple of characters force a scan and merge of
            -- every matching term in the index, which is the cost this table
            -- exists to remove.
            CREATE VIRTUAL TABLE IF NOT EXISTS meetings_fts USING fts5(
                meeting_id UNINDEXED,
                title,
                body,
                prefix='2 3',
                tokenize='unicode61 remove_diacritics 2'
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        // Additive, and the only way to add a column to a table that already
        // holds the user's meetings. SQLite has no `ADD COLUMN IF NOT EXISTS`,
        // and re-running is the normal case — every launch after the first — so
        // the duplicate-column error is the success path on the second run.
        //
        // Only that one. Ignoring every error would let a read-only or full
        // database start the app without the column, and the failure would
        // resurface as `no such column` on the first attempt to open a meeting —
        // far from the thing that actually went wrong.
        if let Err(e) = conn.execute(
            "ALTER TABLE meetings ADD COLUMN title_locked INTEGER NOT NULL DEFAULT 0",
            [],
        ) {
            let message = e.to_string();
            if !message.contains("duplicate column name") {
                return Err(message);
            }
        }
        // Nullable on purpose: NULL means "this meeting never called a paid
        // provider" and renders nothing, which is a different statement from a
        // meeting that ran on a free model and genuinely cost $0.00.
        if let Err(e) = conn.execute("ALTER TABLE meetings ADD COLUMN cost_nano_usd INTEGER", []) {
            let message = e.to_string();
            if !message.contains("duplicate column name") {
                return Err(message);
            }
        }
        // Every section the template asked for, in order, as JSON.
        //
        // A column and not a table: the sections of one meeting are always read
        // and written together, as a whole ordered list, and a table would buy
        // per-section queries nobody has asked for at the price of a join on
        // every open. NULL means a meeting summarised before templates had
        // shapes of their own — the screen falls back to the three columns
        // beside this one, which are still written and still what the search
        // index and the exporter read.
        if let Err(e) = conn.execute("ALTER TABLE meetings ADD COLUMN sections_json TEXT", []) {
            let message = e.to_string();
            if !message.contains("duplicate column name") {
                return Err(message);
            }
        }
        // And on a version, for the same reason a version is the whole set
        // rather than a delta: restoring one that carried only the three
        // columns would erase a client call's Requirements and Risks.
        if let Err(e) = conn.execute(
            "ALTER TABLE summary_versions ADD COLUMN sections_json TEXT",
            [],
        ) {
            let message = e.to_string();
            if !message.contains("duplicate column name") {
                return Err(message);
            }
        }
        // What this meeting calls its two channels. Nullable, and NULL is the
        // normal case rather than a missing value: it means nobody renamed
        // anything here, and the window and the exporter say "Me" and "Others"
        // in the user's own language. Writing those words into the column
        // instead would freeze every meeting in the language it was recorded in.
        if let Err(e) = conn.execute("ALTER TABLE meetings ADD COLUMN speaker_me TEXT", []) {
            let message = e.to_string();
            if !message.contains("duplicate column name") {
                return Err(message);
            }
        }
        if let Err(e) = conn.execute("ALTER TABLE meetings ADD COLUMN speaker_others TEXT", []) {
            let message = e.to_string();
            if !message.contains("duplicate column name") {
                return Err(message);
            }
        }
        // Where in the recording an action item was decided. Nullable, and
        // `NULL` is the ordinary case rather than a missing value: a person who
        // wrote the item has no moment to point at, and a model that ignored the
        // instruction leaves none either. The window shows a stamp when there is
        // one and nothing at all when there is not.
        if let Err(e) = conn.execute("ALTER TABLE action_items ADD COLUMN at_ms INTEGER", []) {
            let message = e.to_string();
            if !message.contains("duplicate column name") {
                return Err(message);
            }
        }
        drop(conn);
        self.backfill_search_index()
    }

    /// Populates the index for meetings that predate it. Runs once: after the
    /// first pass the table is non-empty and every later write maintains it.
    fn backfill_search_index(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let indexed: i64 = conn
            .query_row("SELECT count(*) FROM meetings_fts", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if indexed > 0 {
            return Ok(());
        }
        conn.execute(
            "INSERT INTO meetings_fts (meeting_id, title, body)
             SELECT id, title,
                    transcript_text || char(10) || coalesce(summary,'') || char(10)
                    || coalesce(action_items,'') || char(10) || coalesce(key_points,'')
             FROM meetings",
            [],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Add a provider charge to a meeting's running total.
    ///
    /// The sum happens in SQL rather than by reading the row and writing it
    /// back: two channels transcribe concurrently and a read-modify-write would
    /// silently drop one of them. `COALESCE` is what turns the first charge on a
    /// NULL row into a total instead of into another NULL.
    pub fn add_meeting_cost(&self, id: &str, nano: Option<i64>) -> Result<(), String> {
        let Some(nano) = nano else { return Ok(()) };
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE meetings SET cost_nano_usd = COALESCE(cost_nano_usd, 0) + ?1 WHERE id = ?2",
            params![nano, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_action_items(
        &self,
        meeting_id: &str,
    ) -> Result<Vec<crate::domain::actions::ActionItem>, String> {
        use crate::domain::actions::{ActionItem, ActionSource, ActionStatus};
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, text, owner, due, status, source, edited, at_ms FROM action_items
                 WHERE meeting_id=?1 ORDER BY position, id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![meeting_id], |r| {
                let status: String = r.get(4)?;
                let source: String = r.get(5)?;
                Ok(ActionItem {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    owner: r.get(2)?,
                    due: r.get(3)?,
                    status: if status == "done" {
                        ActionStatus::Done
                    } else {
                        ActionStatus::Open
                    },
                    source: if source == "user" {
                        ActionSource::User
                    } else {
                        ActionSource::Ai
                    },
                    edited: r.get::<_, i64>(6)? != 0,
                    at_ms: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Rewrite the text column that export and search read, from the rows.
    ///
    /// Called inside whatever transaction just changed a row, so the two views
    /// of the list cannot be seen disagreeing.
    fn project_action_text(tx: &rusqlite::Transaction<'_>, meeting_id: &str) -> Result<(), String> {
        let mut stmt = tx
            .prepare("SELECT text FROM action_items WHERE meeting_id=?1 ORDER BY position, id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![meeting_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut lines = Vec::new();
        for r in rows {
            lines.push(format!("- {}", r.map_err(|e| e.to_string())?));
        }
        drop(stmt);
        tx.execute(
            "UPDATE meetings SET action_items=?2 WHERE id=?1",
            params![
                meeting_id,
                lines.join(
                    "
"
                )
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Add one item. Returns its row.
    ///
    /// One item rather than a list, because a client that sends the whole list
    /// sends its idea of every *other* item too — and that idea is stale the
    /// moment anything else writes. Every way that could lose somebody's work
    /// went through exactly that.
    pub fn insert_action_item(
        &self,
        meeting_id: &str,
        item: &crate::domain::actions::ActionItem,
    ) -> Result<i64, String> {
        use crate::domain::actions::{ActionSource, ActionStatus};
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let next: i64 = tx
            .query_row(
                "SELECT coalesce(max(position), -1) + 1 FROM action_items WHERE meeting_id=?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO action_items
               (meeting_id, text, owner, due, status, source, edited, position, at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                meeting_id,
                item.text,
                item.owner,
                item.due,
                match item.status {
                    ActionStatus::Done => "done",
                    ActionStatus::Open => "open",
                },
                match item.source {
                    ActionSource::User => "user",
                    ActionSource::Ai => "ai",
                },
                item.edited as i64,
                next,
                item.at_ms
            ],
        )
        .map_err(|e| e.to_string())?;
        let id = tx.last_insert_rowid();
        Self::project_action_text(&tx, meeting_id)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Change one item, by id and meeting. Marks it touched, which is what
    /// stops the next summary from replacing it.
    pub fn update_action_item(
        &self,
        meeting_id: &str,
        item: &crate::domain::actions::ActionItem,
    ) -> Result<(), String> {
        use crate::domain::actions::ActionStatus;
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        // The count matters. A summary running alongside this can remove an
        // untouched suggestion while its field is being edited, and an UPDATE
        // that matches nothing is not a success — reporting one would hand back
        // a list without the correction and no sign it had been dropped.
        // The citation goes when the words go. It says "this was decided here",
        // and rewriting the task into a different one leaves that pointing at
        // the moment somebody discussed something else — a stamp that plays the
        // wrong words while presenting itself as evidence for the new ones,
        // which is worse than no stamp at all. Kept for an owner, a deadline or
        // a tick, because none of those change what was decided.
        //
        // Compared in SQL rather than read first: a summary running alongside
        // this can rewrite the row between a read and a write, and the version
        // being replaced is the one on disk, not the one this call was built
        // from.
        let changed = tx
            .execute(
                "UPDATE action_items
                    SET text=?3, owner=?4, due=?5, status=?6, edited=1,
                        at_ms = CASE WHEN text=?3 THEN at_ms ELSE NULL END
                  WHERE id=?1 AND meeting_id=?2",
                params![
                    item.id,
                    meeting_id,
                    item.text,
                    item.owner,
                    item.due,
                    match item.status {
                        ActionStatus::Done => "done",
                        ActionStatus::Open => "open",
                    }
                ],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("that task is no longer in this meeting's list".into());
        }
        Self::project_action_text(&tx, meeting_id)?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn delete_action_item(&self, meeting_id: &str, item_id: i64) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM action_items WHERE id=?1 AND meeting_id=?2",
            params![item_id, meeting_id],
        )
        .map_err(|e| e.to_string())?;
        Self::project_action_text(&tx, meeting_id)?;
        tx.commit().map_err(|e| e.to_string())
    }

    /// Replace a meeting's action items with this list, and rewrite the text
    /// column that export and search read from it.
    ///
    /// Only the merge uses this — it genuinely owns the whole list, because it
    /// has just computed it from what was stored.
    ///
    /// One transaction: the rows and the text are two views of one answer, and
    /// a failure between them would leave the list showing one thing and every
    /// export of it showing another.
    pub fn save_action_items(
        &self,
        meeting_id: &str,
        items: &[crate::domain::actions::ActionItem],
    ) -> Result<(), String> {
        use crate::domain::actions::{ActionSource, ActionStatus};
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM action_items WHERE meeting_id=?1",
            params![meeting_id],
        )
        .map_err(|e| e.to_string())?;
        for (position, item) in items.iter().enumerate() {
            tx.execute(
                "INSERT INTO action_items
                   (meeting_id, text, owner, due, status, source, edited, position, at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    meeting_id,
                    item.text,
                    item.owner,
                    item.due,
                    match item.status {
                        ActionStatus::Done => "done",
                        ActionStatus::Open => "open",
                    },
                    match item.source {
                        ActionSource::User => "user",
                        ActionSource::Ai => "ai",
                    },
                    item.edited as i64,
                    position as i64,
                    item.at_ms
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        Self::project_action_text(&tx, meeting_id)?;
        tx.commit().map_err(|e| e.to_string())
    }

    /// Write a corrected transcript, the text the summary reads, and the search
    /// index — as one write or none of them.
    ///
    /// These were three statements in two transactions. A failure between them
    /// left the segments corrected while `transcript_text` still held the old
    /// words, which is the field the model reads: the fix would have been
    /// visible on screen and invisible to every consumer of it. Or it left the
    /// correction saved and unindexed, so search kept answering with what was
    /// misheard.
    pub fn save_corrected_transcript(
        &self,
        m: &MeetingRecord,
        transcript: &LiveTranscript,
    ) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM transcript_segments WHERE meeting_id=?1",
            params![m.id],
        )
        .map_err(|e| e.to_string())?;
        for s in transcript.segments() {
            tx.execute(
                "INSERT INTO transcript_segments (id, meeting_id, speaker, text, start_ms, end_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    s.id,
                    m.id,
                    speaker_str(s.speaker),
                    s.text,
                    s.start_ms as i64,
                    s.end_ms as i64
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "UPDATE meetings SET transcript_text=?2, updated_at=?3 WHERE id=?1",
            params![m.id, m.transcript_text, m.updated_at],
        )
        .map_err(|e| e.to_string())?;
        Self::index_meeting(&tx, m)?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn upsert_meeting(&self, m: &MeetingRecord) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        // The row and its index entry move together. Committing the meeting and
        // then failing to index it leaves search quietly answering with stale
        // content until something else happens to touch the same meeting.
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            r#"INSERT INTO meetings (id, title, status, created_at, updated_at, duration_ms, audio_path,
                transcript_text, summary, action_items, key_points, project, title_locked,
                speaker_me, speaker_others)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
               ON CONFLICT(id) DO UPDATE SET
                title=excluded.title, status=excluded.status, updated_at=excluded.updated_at,
                duration_ms=excluded.duration_ms, audio_path=excluded.audio_path,
                transcript_text=excluded.transcript_text, summary=excluded.summary,
                action_items=excluded.action_items, key_points=excluded.key_points,
                project=excluded.project, title_locked=excluded.title_locked,
                speaker_me=excluded.speaker_me, speaker_others=excluded.speaker_others"#,
            params![
                m.id,
                m.title,
                status_str(m.status),
                m.created_at,
                m.updated_at,
                m.duration_ms as i64,
                m.audio_path,
                m.transcript_text,
                m.summary,
                m.action_items,
                m.key_points,
                m.project,
                m.title_locked as i64,
                m.speaker_me,
                m.speaker_others,
            ],
        )
        .map_err(|e| e.to_string())?;
        Self::index_meeting(&tx, m)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_meeting(&self, id: &str) -> Result<Option<MeetingRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, status, created_at, updated_at, duration_ms, audio_path,
                        transcript_text, summary, action_items, key_points, project, title_locked,
                        cost_nano_usd, sections_json, speaker_me, speaker_others
                 FROM meetings WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            Ok(Some(row_to_meeting(row)?))
        } else {
            Ok(None)
        }
    }

    /// The sidebar list. `transcript_text` comes back empty on purpose: it is the
    /// largest column by far, nothing in the UI reads it from this payload, and
    /// shipping every transcript across the IPC boundary to render a list of
    /// titles was the single most expensive thing the app did on startup.
    pub fn list_meetings(&self) -> Result<Vec<MeetingRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, status, created_at, updated_at, duration_ms, audio_path,
                        '' AS transcript_text, summary, action_items, key_points, project, title_locked,
                        cost_nano_usd, sections_json, speaker_me, speaker_others
                 FROM meetings ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            // row_to_meeting returns our own error type, so it cannot be raised
            // from inside the closure; collecting Results keeps a malformed row from
            // panicking the command.
            .query_map([], |row| Ok(row_to_meeting(row)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())??);
        }
        Ok(out)
    }

    pub fn delete_meeting(&self, id: &str) -> Result<(), String> {
        // The recording goes first, and its failure aborts the whole delete.
        //
        // Removing the rows first would destroy `audio_path`, so a WAV that could
        // not be deleted — file locked, permissions — would become unreachable by
        // any later attempt: the user is told the meeting is gone while a private
        // recording sits on disk with nothing left pointing at it. Failing here
        // leaves everything intact and the delete repeatable.
        if let Some(audio) = self.get_meeting(id)?.and_then(|m| m.audio_path) {
            delete_recording(Path::new(&audio))?;
        }
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        // One transaction: the audio is already gone by this point, so a partial
        // delete would leave a meeting the user cannot play and cannot finish
        // removing. All four rows go together or none do, and the retry works
        // because a missing file is not an error.
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM meetings_fts WHERE meeting_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM transcript_segments WHERE meeting_id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM chat_messages WHERE meeting_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM summary_versions WHERE meeting_id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM meetings WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Every meeting, removed one at a time through the path a single delete
    /// takes.
    ///
    /// Not `DELETE FROM meetings`: that empties the tables and leaves every
    /// recording on disk, which is the half of a wipe that actually matters.
    /// Going one by one reuses the ordering that refuses to drop a row whose
    /// audio could not be removed, so a locked file costs that meeting and
    /// nothing else.
    ///
    /// Returns how many were removed. A partial wipe is reported as an error
    /// naming what is left rather than as success: a user who asked for
    /// everything to be gone has to be told when some of it is not.
    pub fn delete_all_meetings(&self) -> Result<usize, String> {
        let ids: Vec<String> = self.list_meetings()?.into_iter().map(|m| m.id).collect();
        let total = ids.len();
        let mut done = 0usize;
        let mut first_error = None;
        for id in ids {
            match self.delete_meeting(&id) {
                Ok(()) => done += 1,
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }
        match first_error {
            None => Ok(done),
            Some(e) => Err(format!(
                "{done} of {total} meetings were deleted. The rest are still on this computer: {e}"
            )),
        }
    }

    pub fn save_transcript(
        &self,
        meeting_id: &str,
        transcript: &LiveTranscript,
        names: &SpeakerNames,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM transcript_segments WHERE meeting_id=?1",
            params![meeting_id],
        )
        .map_err(|e| e.to_string())?;
        for s in transcript.segments() {
            conn.execute(
                "INSERT INTO transcript_segments (id, meeting_id, speaker, text, start_ms, end_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    s.id,
                    meeting_id,
                    speaker_str(s.speaker),
                    s.text,
                    s.start_ms as i64,
                    s.end_ms as i64
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute(
            "UPDATE meetings SET transcript_text=?1, updated_at=?2 WHERE id=?3",
            params![
                transcript.plain_text(names),
                chrono::Utc::now().to_rfc3339(),
                meeting_id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_transcript(&self, meeting_id: &str) -> Result<LiveTranscript, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, speaker, text, start_ms, end_ms FROM transcript_segments
                 WHERE meeting_id=?1 ORDER BY start_ms ASC",
            )
            .map_err(|e| e.to_string())?;
        let mut t = LiveTranscript::new();
        let rows = stmt
            .query_map(params![meeting_id], |row| {
                Ok(TranscriptSegment {
                    id: row.get(0)?,
                    speaker: parse_speaker(&row.get::<_, String>(1)?),
                    text: row.get(2)?,
                    start_ms: row.get::<_, i64>(3)? as u64,
                    end_ms: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            t.append(r.map_err(|e| e.to_string())?);
        }
        Ok(t)
    }

    /// Append the current insights as the next version of a meeting.
    ///
    /// Append-only: nothing here overwrites a version, and the meeting row is
    /// always a mirror of the newest one, which is what leaves search, export and
    /// the existing summary path untouched.
    pub fn push_summary_version(
        &self,
        meeting_id: &str,
        origin: &str,
        insights: &MeetingInsights,
    ) -> Result<SummaryVersion, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let next: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM summary_versions WHERE meeting_id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let created_at = chrono::Utc::now().to_rfc3339();
        let (key_points, action_items) = (insights.key_points_text(), insights.action_items_text());
        let sections_json = if insights.sections.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&insights.sections).map_err(|e| e.to_string())?)
        };
        conn.execute(
            "INSERT INTO summary_versions
                (meeting_id, version, origin, created_at, summary, key_points, action_items, sections_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                meeting_id,
                next,
                origin,
                created_at,
                insights.summary,
                key_points,
                action_items,
                sections_json
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(SummaryVersion {
            version: next,
            origin: origin.to_string(),
            created_at,
            summary: insights.summary.clone(),
            key_points,
            action_items,
            sections_json,
        })
    }

    pub fn list_summary_versions(&self, meeting_id: &str) -> Result<Vec<SummaryVersion>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT version, origin, created_at, summary, key_points, action_items, sections_json
                 FROM summary_versions WHERE meeting_id = ?1 ORDER BY version ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![meeting_id], |r| {
                Ok(SummaryVersion {
                    version: r.get(0)?,
                    origin: r.get(1)?,
                    created_at: r.get(2)?,
                    summary: r.get(3)?,
                    key_points: r.get(4)?,
                    action_items: r.get(5)?,
                    sections_json: r.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn save_insights(
        &self,
        meeting_id: &str,
        insights: &MeetingInsights,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        // NULL rather than `[]` when there are none, so a meeting summarised by
        // a path that has no sections is indistinguishable from one summarised
        // before they existed — both fall back to the three columns.
        let sections = if insights.sections.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&insights.sections).map_err(|e| e.to_string())?)
        };
        conn.execute(
            "UPDATE meetings SET summary=?1, action_items=?2, key_points=?3, sections_json=?4, updated_at=?5 WHERE id=?6",
            params![
                insights.summary,
                insights.action_items_text(),
                insights.key_points_text(),
                sections,
                chrono::Utc::now().to_rfc3339(),
                meeting_id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn add_context_note(
        &self,
        meeting_id: &str,
        text: &str,
        at_ms: Option<i64>,
    ) -> Result<crate::domain::context::ContextNote, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO context_notes (meeting_id, text, at_ms, created_at) VALUES (?1,?2,?3,?4)",
            params![meeting_id, text, at_ms, created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(crate::domain::context::ContextNote {
            id: conn.last_insert_rowid(),
            text: text.to_string(),
            at_ms,
            created_at,
        })
    }

    /// In the order they were written, which is the order they were meant in.
    /// By `id` rather than by `at_ms`: a note added after the recording has no
    /// offset, and sorting on a null would move it somewhere it never was.
    pub fn list_context_notes(
        &self,
        meeting_id: &str,
    ) -> Result<Vec<crate::domain::context::ContextNote>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, text, at_ms, created_at FROM context_notes
                 WHERE meeting_id=?1 ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![meeting_id], |r| {
                Ok(crate::domain::context::ContextNote {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    at_ms: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Scoped to the meeting as well as the id: the id arrives from the WebView,
    /// and a note belonging to another meeting must not be deletable by guessing
    /// a number.
    pub fn delete_context_note(&self, meeting_id: &str, note_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM context_notes WHERE id=?1 AND meeting_id=?2",
            params![note_id, meeting_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn add_chat(&self, meeting_id: &str, role: &str, content: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO chat_messages (meeting_id, role, content, created_at) VALUES (?1,?2,?3,?4)",
            params![meeting_id, role, content, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_chat(
        &self,
        meeting_id: &str,
    ) -> Result<Vec<crate::domain::chat::ChatMessage>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT role, content FROM chat_messages WHERE meeting_id=?1 ORDER BY id ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![meeting_id], |row| {
                Ok(crate::domain::chat::ChatMessage {
                    role: row.get(0)?,
                    content: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        let Some(match_expr) = fts_match_expression(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT meeting_id, title,
                        snippet(meetings_fts, 2, '', '', '…', 12),
                        bm25(meetings_fts)
                 FROM meetings_fts
                 WHERE meetings_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![match_expr, limit as i64], |row| {
                Ok(SearchHit {
                    meeting_id: row.get(0)?,
                    title: row.get(1)?,
                    snippet: row.get::<_, String>(2)?.trim().to_string(),
                    // bm25 is negative and better the lower it is; the UI wants
                    // "higher is more relevant".
                    score: -row.get::<_, f64>(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    fn index_meeting(conn: &rusqlite::Transaction<'_>, m: &MeetingRecord) -> Result<(), String> {
        conn.execute(
            "DELETE FROM meetings_fts WHERE meeting_id=?1",
            params![m.id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO meetings_fts (meeting_id, title, body) VALUES (?1,?2,?3)",
            params![
                m.id,
                m.title,
                format!(
                    "{}\n{}\n{}\n{}\n{}",
                    m.transcript_text,
                    m.summary.clone().unwrap_or_default(),
                    m.action_items.clone().unwrap_or_default(),
                    m.key_points.clone().unwrap_or_default(),
                    // The template's own sections. Without them a client call's
                    // Requirements and Risks are stored, rendered and exported
                    // but cannot be found — and search is how a meeting from
                    // three weeks ago is reached at all. Bodies only: the keys
                    // are `action_items`, and indexing those would match every
                    // meeting on a search for "action".
                    m.sections
                        .iter()
                        .map(|s| s.body.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Persists settings, dropping the API key when the keychain is holding it.
    pub fn save_settings_with(&self, settings: &AppSettings, home: KeyHome) -> Result<(), String> {
        let mut on_disk = settings.clone();
        if home == KeyHome::Keychain {
            on_disk.openrouter_api_key = None;
        }
        let json = serde_json::to_string(&on_disk).map_err(|e| e.to_string())?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('app', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// An API key left in the settings row by an older build, if any.
    ///
    /// Reading and clearing are separate on purpose: clearing before the keychain
    /// has accepted the value would destroy the user's key whenever the keychain
    /// is unavailable.
    pub fn legacy_api_key(&self) -> Result<Option<String>, String> {
        let Some(value) = self.raw_settings_json()? else {
            return Ok(None);
        };
        Ok(value
            .get("openrouter_api_key")
            .and_then(|k| k.as_str())
            .map(str::to_string)
            .filter(|k| !k.trim().is_empty()))
    }

    /// Drops the plaintext key from the settings row. Safe to call repeatedly.
    pub fn clear_legacy_api_key(&self) -> Result<(), String> {
        let Some(mut value) = self.raw_settings_json()? else {
            return Ok(());
        };
        if value.get("openrouter_api_key").map(|k| k.is_null()) == Some(true) {
            return Ok(());
        }
        value["openrouter_api_key"] = serde_json::Value::Null;
        let cleaned = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE settings SET value=?1 WHERE key='app'",
            params![cleaned],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn raw_settings_json(&self) -> Result<Option<serde_json::Value>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key='app'")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let Some(row) = rows.next().map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let raw: String = row.get(0).map_err(|e| e.to_string())?;
        Ok(serde_json::from_str(&raw).ok())
    }

    pub fn load_settings(&self) -> Result<AppSettings, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key='app'")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let v: String = row.get(0).map_err(|e| e.to_string())?;
            let settings: AppSettings = serde_json::from_str(&v).map_err(|e| e.to_string())?;
            // Not migrated here. A row naming a retired model is repointed
            // once, in `AppState::new`, which is the only place that can
            // also tell the user their summaries changed hands.
            Ok(settings)
        } else {
            Ok(AppSettings::default())
        }
    }
}

fn status_str(s: MeetingStatus) -> &'static str {
    match s {
        MeetingStatus::Idle => "idle",
        MeetingStatus::Recording => "recording",
        MeetingStatus::Paused => "paused",
        MeetingStatus::Transcribing => "transcribing",
        MeetingStatus::Summarizing => "summarizing",
        MeetingStatus::Ready => "ready",
        MeetingStatus::Failed => "failed",
    }
}

fn parse_status(s: &str) -> MeetingStatus {
    match s {
        "recording" => MeetingStatus::Recording,
        "paused" => MeetingStatus::Paused,
        "transcribing" => MeetingStatus::Transcribing,
        "summarizing" => MeetingStatus::Summarizing,
        "ready" => MeetingStatus::Ready,
        "failed" => MeetingStatus::Failed,
        _ => MeetingStatus::Idle,
    }
}

fn speaker_str(s: Speaker) -> &'static str {
    match s {
        Speaker::Me => "me",
        Speaker::Others => "others",
    }
}

fn parse_speaker(s: &str) -> Speaker {
    match s {
        "others" => Speaker::Others,
        _ => Speaker::Me,
    }
}

fn row_to_meeting(row: &rusqlite::Row<'_>) -> Result<MeetingRecord, String> {
    Ok(MeetingRecord {
        id: row.get(0).map_err(|e| e.to_string())?,
        title: row.get(1).map_err(|e| e.to_string())?,
        status: parse_status(&row.get::<_, String>(2).map_err(|e| e.to_string())?),
        created_at: row.get(3).map_err(|e| e.to_string())?,
        updated_at: row.get(4).map_err(|e| e.to_string())?,
        duration_ms: row.get::<_, i64>(5).map_err(|e| e.to_string())? as u64,
        audio_path: row.get(6).map_err(|e| e.to_string())?,
        transcript_text: row.get(7).map_err(|e| e.to_string())?,
        summary: row.get(8).map_err(|e| e.to_string())?,
        action_items: row.get(9).map_err(|e| e.to_string())?,
        key_points: row.get(10).map_err(|e| e.to_string())?,
        // A row written before the column existed reads NULL; one written by a
        // build whose section shapes have since changed reads JSON this build
        // may not recognise. Both answer with an empty list, and the screen
        // falls back to the three columns above — the alternative is failing to
        // open a meeting over the shape of its headings.
        sections: row
            .get::<_, Option<String>>(14)
            .map_err(|e| e.to_string())?
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default(),
        project: row.get(11).map_err(|e| e.to_string())?,
        title_locked: row.get::<_, i64>(12).map_err(|e| e.to_string())? != 0,
        cost_nano_usd: row.get(13).map_err(|e| e.to_string())?,
        // Derived on read. Formatting money is one decision and it belongs on
        // the side that owns the number, not repeated in the window.
        cost_label: row
            .get::<_, Option<i64>>(13)
            .map_err(|e| e.to_string())?
            .map(crate::domain::cost::format_cost),
        speaker_me: row.get(15).map_err(|e| e.to_string())?,
        speaker_others: row.get(16).map_err(|e| e.to_string())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::i18n::Locale;
    use crate::domain::job::MeetingStatus;
    use tempfile::tempdir;

    fn english() -> SpeakerNames {
        SpeakerNames::resolve(None, None, Locale::En)
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let m = MeetingRecord {
            id: "m1".into(),
            title: "Sync".into(),
            status: MeetingStatus::Ready,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            duration_ms: 1200,
            audio_path: Some("a.wav".into()),
            transcript_text: "Me: hello".into(),
            summary: Some("hi".into()),
            action_items: None,
            key_points: None,
            sections: Vec::new(),
            project: Some("Core".into()),
            title_locked: false,
            cost_nano_usd: None,
            cost_label: None,
            speaker_me: None,
            speaker_others: None,
        };
        db.upsert_meeting(&m).unwrap();
        let mut t = LiveTranscript::new();
        t.append(TranscriptSegment::new(Speaker::Me, "hello", 0, 500));
        db.save_transcript("m1", &t, &english()).unwrap();
        let loaded = db.get_meeting("m1").unwrap().unwrap();
        assert_eq!(loaded.title, "Sync");
        assert_eq!(loaded.status, MeetingStatus::Ready);
        let t2 = db.load_transcript("m1").unwrap();
        assert_eq!(t2.segments().len(), 1);
        assert_eq!(t2.segments()[0].text, "hello");
        let hits = db.search("hello", 5).unwrap();
        assert_eq!(hits.len(), 1);
        let settings = AppSettings {
            language: "pt".into(),
            ..Default::default()
        };
        db.save_settings_with(&settings, KeyHome::Keychain).unwrap();
        let s2 = db.load_settings().unwrap();
        assert_eq!(s2.language, "pt");
    }

    #[test]
    fn the_api_key_never_reaches_the_database() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let settings = AppSettings {
            openrouter_api_key: Some("sk-or-must-not-be-written".into()),
            ..Default::default()
        };
        db.save_settings_with(&settings, KeyHome::Keychain).unwrap();

        let stored: String = {
            let conn = db.conn.lock().unwrap();
            conn.query_row("SELECT value FROM settings WHERE key='app'", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert!(
            !stored.contains("sk-or-must-not-be-written"),
            "settings row still carries the key: {stored}"
        );
        assert!(db.load_settings().unwrap().openrouter_api_key.is_none());
    }

    #[test]
    fn keep_in_row_leaves_the_key_where_the_keychain_could_not_take_it() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let settings = AppSettings {
            openrouter_api_key: Some("sk-or-nowhere-else-to-go".into()),
            ..Default::default()
        };

        // The keychain refused. Stripping the row here would leave the user's key
        // stored nowhere at all.
        db.save_settings_with(&settings, KeyHome::KeepInRow)
            .unwrap();
        assert_eq!(
            db.legacy_api_key().unwrap().as_deref(),
            Some("sk-or-nowhere-else-to-go")
        );

        // Once the keychain takes it, the row lets go.
        db.save_settings_with(&settings, KeyHome::Keychain).unwrap();
        assert_eq!(db.legacy_api_key().unwrap(), None);
    }

    #[test]
    fn a_key_left_by_an_older_build_is_taken_out_of_the_row() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();

        // Exactly what an older build wrote: the whole struct, key included.
        let legacy = AppSettings {
            openrouter_api_key: Some("sk-or-legacy".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&legacy).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('app', ?1)",
                params![json],
            )
            .unwrap();
        }

        assert_eq!(
            db.legacy_api_key().unwrap().as_deref(),
            Some("sk-or-legacy")
        );

        // Reading must not destroy it: the keychain write can still fail.
        assert_eq!(
            db.legacy_api_key().unwrap().as_deref(),
            Some("sk-or-legacy")
        );

        db.clear_legacy_api_key().unwrap();
        let stored: String = {
            let conn = db.conn.lock().unwrap();
            conn.query_row("SELECT value FROM settings WHERE key='app'", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert!(!stored.contains("sk-or-legacy"), "key survived: {stored}");

        // Idempotent: a second start finds nothing left to migrate.
        assert_eq!(db.legacy_api_key().unwrap(), None);
        db.clear_legacy_api_key().unwrap();
    }

    #[test]
    fn search_finds_a_meeting_by_its_transcript() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let mut m = sample_meeting("m1", "Sprint planning");
        m.transcript_text = "we agreed to cut the reporting module".into();
        db.upsert_meeting(&m).unwrap();

        let hits = db.search("reporting", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, "m1");
        assert!(!hits[0].snippet.is_empty());

        assert!(db.search("nothingmatchesthis", 10).unwrap().is_empty());
    }

    #[test]
    fn search_treats_fts_operators_as_plain_text() {
        // These are FTS5 syntax. Passed through raw they raise a query error
        // instead of searching, which is what a user typing normally would hit.
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let mut m = sample_meeting("m1", "Budget");
        m.transcript_text = "the budget for Q3 was approved".into();
        db.upsert_meeting(&m).unwrap();

        for q in [
            "budget: Q3",
            "budget*",
            "budget AND",
            "budget \"quoted",
            "(budget)",
        ] {
            let hits = db.search(q, 10);
            assert!(hits.is_ok(), "query {q:?} errored: {:?}", hits.err());
        }
        assert_eq!(db.search("budget: Q3", 10).unwrap().len(), 1);
    }

    #[test]
    fn search_index_follows_edits_and_deletes() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let mut m = sample_meeting("m1", "Retro");
        m.transcript_text = "original wording".into();
        db.upsert_meeting(&m).unwrap();
        assert_eq!(db.search("original", 10).unwrap().len(), 1);

        m.transcript_text = "replaced wording".into();
        db.upsert_meeting(&m).unwrap();
        assert!(db.search("original", 10).unwrap().is_empty(), "stale index");
        assert_eq!(db.search("replaced", 10).unwrap().len(), 1);

        db.delete_meeting("m1").unwrap();
        assert!(db.search("replaced", 10).unwrap().is_empty());
    }

    #[test]
    fn the_list_payload_leaves_out_transcripts() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let mut m = sample_meeting("m1", "Long one");
        m.transcript_text = "a very long transcript".into();
        db.upsert_meeting(&m).unwrap();

        assert_eq!(db.list_meetings().unwrap()[0].transcript_text, "");
        // Still there when the meeting is actually opened.
        assert_eq!(
            db.get_meeting("m1").unwrap().unwrap().transcript_text,
            "a very long transcript"
        );
    }

    /// The reason the sum is in SQL. Two channels transcribe at once, and a
    /// read-modify-write in Rust would let one charge overwrite the other.
    #[test]
    fn charges_accumulate_and_a_stale_upsert_cannot_roll_them_back() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let m = sample_meeting("m1", "One");
        db.upsert_meeting(&m).unwrap();

        // Nothing charged yet: no price at all, which is not the same as zero.
        assert_eq!(db.get_meeting("m1").unwrap().unwrap().cost_nano_usd, None);

        db.add_meeting_cost("m1", Some(508_000)).unwrap();
        db.add_meeting_cost("m1", Some(492_000)).unwrap();
        // A local chunk reports nothing and must not reset the total.
        db.add_meeting_cost("m1", None).unwrap();
        let after = db.get_meeting("m1").unwrap().unwrap();
        assert_eq!(after.cost_nano_usd, Some(1_000_000));
        assert_eq!(after.cost_label.as_deref(), Some("$0.0010"));

        // `m` is the pre-charge snapshot a caller may still be holding. Writing
        // it back must not undo what was billed in between.
        db.upsert_meeting(&m).unwrap();
        assert_eq!(
            db.get_meeting("m1").unwrap().unwrap().cost_nano_usd,
            Some(1_000_000),
            "an upsert carrying a stale record must not roll a charge back"
        );
    }

    /// History is append-only and numbered from one per meeting, and deleting the
    /// meeting takes it with it.
    #[test]
    fn versions_accumulate_and_leave_with_the_meeting() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.upsert_meeting(&sample_meeting("m1", "One")).unwrap();
        db.upsert_meeting(&sample_meeting("m2", "Two")).unwrap();

        let first = MeetingInsights {
            summary: "s1".into(),
            key_points: vec!["a".into()],
            action_items: vec![],
            sections: Vec::new(),
        };
        let second = MeetingInsights {
            summary: "s1".into(),
            key_points: vec!["a".into(), "b".into()],
            action_items: vec![],
            sections: Vec::new(),
        };
        let v1 = db.push_summary_version("m1", "summarize", &first).unwrap();
        let v2 = db
            .push_summary_version("m1", "key_points", &second)
            .unwrap();
        assert_eq!((v1.version, v2.version), (1, 2));

        // Numbering is per meeting, not global.
        assert_eq!(
            db.push_summary_version("m2", "summarize", &first)
                .unwrap()
                .version,
            1
        );

        let all = db.list_summary_versions("m1").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].key_points, "- a", "the first version is still there");
        assert_eq!(all[1].origin, "key_points");

        db.delete_meeting("m1").unwrap();
        assert!(db.list_summary_versions("m1").unwrap().is_empty());
        assert_eq!(
            db.list_summary_versions("m2").unwrap().len(),
            1,
            "another meeting's history must survive"
        );
    }

    fn sample_meeting(id: &str, title: &str) -> MeetingRecord {
        MeetingRecord {
            id: id.into(),
            title: title.into(),
            status: MeetingStatus::Ready,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            duration_ms: 1000,
            audio_path: None,
            transcript_text: String::new(),
            summary: None,
            action_items: None,
            key_points: None,
            sections: Vec::new(),
            title_locked: false,
            cost_nano_usd: None,
            cost_label: None,
            project: None,
            speaker_me: None,
            speaker_others: None,
        }
    }

    /// The citation says "this was decided here". Rewriting the task into a
    /// different one leaves it pointing at the moment somebody discussed
    /// something else, and a stamp that plays the wrong words while presenting
    /// itself as evidence for the new ones is worse than no stamp at all.
    #[test]
    fn editing_the_words_of_an_item_drops_its_citation() {
        use crate::domain::actions::{ActionItem, ActionSource, ActionStatus};
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.upsert_meeting(&sample_meeting("m1", "Sync")).unwrap();
        db.insert_action_item(
            "m1",
            &ActionItem {
                id: 0,
                text: "Assinar o build".into(),
                owner: None,
                due: None,
                status: ActionStatus::Open,
                source: ActionSource::Ai,
                edited: false,
                at_ms: Some(45_000),
            },
        )
        .unwrap();
        let stored = db.list_action_items("m1").unwrap().remove(0);
        assert_eq!(stored.at_ms, Some(45_000));

        // An owner, a deadline and a tick change none of what was decided.
        db.update_action_item(
            "m1",
            &ActionItem {
                owner: Some("Ana".into()),
                due: Some("sexta".into()),
                status: ActionStatus::Done,
                ..stored.clone()
            },
        )
        .unwrap();
        assert_eq!(
            db.list_action_items("m1").unwrap()[0].at_ms,
            Some(45_000),
            "the task is the same task"
        );

        // The words do.
        db.update_action_item(
            "m1",
            &ActionItem {
                text: "Publicar as notas".into(),
                ..stored
            },
        )
        .unwrap();
        assert_eq!(db.list_action_items("m1").unwrap()[0].at_ms, None);
    }

    /// The columns arrive on a database that already holds somebody's meetings,
    /// and `migrate` runs again on every launch after the first — so the second
    /// open is the normal case, not the edge one.
    #[test]
    fn naming_the_channels_is_additive_and_survives_a_reopen() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let mut existing = sample_meeting("m1", "Sync");
        existing.transcript_text = "Me: hello".into();
        db.upsert_meeting(&existing).unwrap();
        drop(db);

        let db = Database::open(dir.path()).unwrap();
        let loaded = db.get_meeting("m1").unwrap().unwrap();
        assert_eq!(loaded.speaker_me, None, "nobody renamed this one");
        assert_eq!(
            loaded.transcript_text, "Me: hello",
            "and its words are its own"
        );

        // A name belongs to one meeting. The other keeps reading as it did.
        let mut named = sample_meeting("m2", "Client call");
        named.speaker_me = Some("Ana".into());
        named.speaker_others = Some("Cliente".into());
        db.upsert_meeting(&named).unwrap();
        assert_eq!(
            db.get_meeting("m2").unwrap().unwrap().speaker_me.as_deref(),
            Some("Ana")
        );
        assert_eq!(db.get_meeting("m1").unwrap().unwrap().speaker_me, None);
        let listed = db.list_meetings().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed
            .iter()
            .any(|m| m.speaker_others.as_deref() == Some("Cliente")));
    }

    #[test]
    fn a_multi_word_search_still_matches_on_any_term() {
        // The search this replaced returned a meeting when any term appeared and
        // used the rest only for ranking. Joining terms with FTS5's implicit AND
        // would have made this two-word query find nothing.
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let mut m = sample_meeting("m1", "Budget");
        m.transcript_text = "the budget was approved".into();
        db.upsert_meeting(&m).unwrap();

        assert_eq!(db.search("budget Q3", 10).unwrap().len(), 1);
    }

    #[test]
    fn deleting_a_meeting_removes_its_recording() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let recordings = crate::paths::recordings_dir();
        std::fs::create_dir_all(&recordings).unwrap();
        let wav = recordings.join("delete-me-test.wav");
        std::fs::write(&wav, vec![0u8; 16]).unwrap();

        let mut m = sample_meeting("m1", "With audio");
        m.audio_path = Some(wav.display().to_string());
        db.upsert_meeting(&m).unwrap();
        db.delete_meeting("m1").unwrap();

        assert!(!wav.exists(), "the recording outlived the meeting");
        assert!(db.get_meeting("m1").unwrap().is_none());
    }

    #[test]
    fn a_recording_outside_the_app_directory_is_left_alone() {
        // A tampered audio_path must not turn deletion into an arbitrary file
        // removal. Path::starts_with alone would accept a traversal like this.
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let outsider = dir.path().join("not-ours.wav");
        std::fs::write(&outsider, b"someone else's file").unwrap();
        let traversal = crate::paths::recordings_dir()
            .join("..")
            .join("..")
            .join(outsider.file_name().unwrap());

        let mut m = sample_meeting("m1", "Tampered");
        m.audio_path = Some(traversal.display().to_string());
        db.upsert_meeting(&m).unwrap();
        db.delete_meeting("m1").unwrap();

        assert!(outsider.exists(), "delete escaped the recordings directory");
    }

    #[test]
    fn search_matches_a_prefix_as_the_user_types() {
        // Search runs on every keystroke, so a query has to match before the word
        // is finished. A bare FTS5 phrase only matches whole tokens.
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let m = sample_meeting("m1", "Sprint planning");
        db.upsert_meeting(&m).unwrap();

        for typed in ["p", "pl", "plan", "planning"] {
            assert_eq!(db.search(typed, 10).unwrap().len(), 1, "typed {typed:?}");
        }
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let on: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(on, 1, "cascade deletes are decorative without this pragma");
    }
}
