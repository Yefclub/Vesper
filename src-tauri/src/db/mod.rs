use crate::domain::job::{MeetingRecord, MeetingStatus};
use crate::domain::search::{search_meetings, SearchDocument, SearchHit};
use crate::domain::settings::AppSettings;
use crate::domain::summary::MeetingInsights;
use crate::domain::transcript::{LiveTranscript, TranscriptSegment};
use crate::domain::speaker::Speaker;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
    data_dir: PathBuf,
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
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_segments_meeting ON transcript_segments(meeting_id);
            CREATE INDEX IF NOT EXISTS idx_chat_meeting ON chat_messages(meeting_id);
            "#,
        )
        .map_err(|e| e.to_string())
    }

    pub fn upsert_meeting(&self, m: &MeetingRecord) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            r#"INSERT INTO meetings (id, title, status, created_at, updated_at, duration_ms, audio_path,
                transcript_text, summary, action_items, key_points, project)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
               ON CONFLICT(id) DO UPDATE SET
                title=excluded.title, status=excluded.status, updated_at=excluded.updated_at,
                duration_ms=excluded.duration_ms, audio_path=excluded.audio_path,
                transcript_text=excluded.transcript_text, summary=excluded.summary,
                action_items=excluded.action_items, key_points=excluded.key_points,
                project=excluded.project"#,
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
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_meeting(&self, id: &str) -> Result<Option<MeetingRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, status, created_at, updated_at, duration_ms, audio_path,
                        transcript_text, summary, action_items, key_points, project
                 FROM meetings WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query(params![id]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            Ok(Some(row_to_meeting(&row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_meetings(&self) -> Result<Vec<MeetingRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, status, created_at, updated_at, duration_ms, audio_path,
                        transcript_text, summary, action_items, key_points, project
                 FROM meetings ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok(row_to_meeting(row).unwrap()))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn delete_meeting(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM transcript_segments WHERE meeting_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM chat_messages WHERE meeting_id=?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM meetings WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn save_transcript(&self, meeting_id: &str, transcript: &LiveTranscript) -> Result<(), String> {
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
                transcript.plain_text(),
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

    pub fn save_insights(&self, meeting_id: &str, insights: &MeetingInsights) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE meetings SET summary=?1, action_items=?2, key_points=?3, updated_at=?4 WHERE id=?5",
            params![
                insights.summary,
                insights.action_items_text(),
                insights.key_points_text(),
                chrono::Utc::now().to_rfc3339(),
                meeting_id
            ],
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

    pub fn list_chat(&self, meeting_id: &str) -> Result<Vec<crate::domain::chat::ChatMessage>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT role, content FROM chat_messages WHERE meeting_id=?1 ORDER BY id ASC",
            )
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
        let meetings = self.list_meetings()?;
        let docs: Vec<SearchDocument> = meetings
            .into_iter()
            .map(|m| SearchDocument {
                meeting_id: m.id,
                title: m.title,
                body: format!(
                    "{}\n{}\n{}\n{}",
                    m.transcript_text,
                    m.summary.unwrap_or_default(),
                    m.action_items.unwrap_or_default(),
                    m.key_points.unwrap_or_default()
                ),
            })
            .collect();
        Ok(search_meetings(&docs, query, limit))
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let json = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('app', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_settings(&self) -> Result<AppSettings, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key='app'")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let v: String = row.get(0).map_err(|e| e.to_string())?;
            serde_json::from_str(&v).map_err(|e| e.to_string())
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
        project: row.get(11).map_err(|e| e.to_string())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::MeetingStatus;
    use tempfile::tempdir;

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
            project: Some("Core".into()),
        };
        db.upsert_meeting(&m).unwrap();
        let mut t = LiveTranscript::new();
        t.append(TranscriptSegment::new(Speaker::Me, "hello", 0, 500));
        db.save_transcript("m1", &t).unwrap();
        let loaded = db.get_meeting("m1").unwrap().unwrap();
        assert_eq!(loaded.title, "Sync");
        assert_eq!(loaded.status, MeetingStatus::Ready);
        let t2 = db.load_transcript("m1").unwrap();
        assert_eq!(t2.segments().len(), 1);
        assert_eq!(t2.segments()[0].text, "hello");
        let hits = db.search("hello", 5).unwrap();
        assert_eq!(hits.len(), 1);
        let mut settings = AppSettings::default();
        settings.openrouter_api_key = Some("sk-test".into());
        db.save_settings(&settings).unwrap();
        let s2 = db.load_settings().unwrap();
        assert_eq!(s2.openrouter_api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let on: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(on, 1, "cascade deletes are decorative without this pragma");
    }
}
