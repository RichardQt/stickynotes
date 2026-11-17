//! PlumStore: read/write access to Windows Sticky Notes' plum.sqlite.

pub mod backup;
pub mod model;

use crate::ticks;
use model::NoteSummary;
use rusqlite::{params, Connection, OpenFlags};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PlumError {
    #[error("plum.sqlite not found (is Microsoft Sticky Notes installed?)")]
    NotFound,
    #[error("Sticky Notes app is running; close it before writing (concurrent writes can corrupt the database)")]
    AppRunning,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, PlumError>;

/// Locate plum.sqlite under %LOCALAPPDATA%\Packages, with glob fallback.
pub fn find_plum_path() -> Result<PathBuf> {
    let local = std::env::var("LOCALAPPDATA")
        .map_err(|_| PlumError::Other("LOCALAPPDATA not set".into()))?;
    let packages = Path::new(&local).join("Packages");
    let direct = packages
        .join("Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe")
        .join("LocalState")
        .join("plum.sqlite");
    if direct.exists() {
        return Ok(direct);
    }
    // Glob fallback for other package suffixes
    if let Ok(entries) = std::fs::read_dir(&packages) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("Microsoft.MicrosoftStickyNotes_") {
                let p = e.path().join("LocalState").join("plum.sqlite");
                if p.exists() {
                    return Ok(p);
                }
            }
        }
    }
    Err(PlumError::NotFound)
}

pub struct PlumStore {
    path: PathBuf,
}

impl PlumStore {
    pub fn open() -> Result<Self> {
        Ok(Self { path: find_plum_path()? })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn conn_read(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        Ok(conn)
    }

    fn conn_write(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        Ok(conn)
    }

    /// List active (non-deleted) notes, newest first.
    pub fn list_notes(&self) -> Result<Vec<NoteSummary>> {
        let conn = self.conn_read()?;
        let mut stmt = conn.prepare(
            "SELECT Id, Text, Theme, CreatedAt, UpdatedAt FROM Note \
             WHERE DeletedAt IS NULL ORDER BY UpdatedAt DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let text: String = row.get(1)?;
            let (title, snippet) = extract_title_and_snippet(&text);
            Ok(NoteSummary {
                id: row.get(0)?,
                title,
                snippet,
                theme: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    /// Load one note's full plum text.
    pub fn get_note_text(&self, id: &str) -> Result<Option<String>> {
        let conn = self.conn_read()?;
        let mut stmt = conn.prepare("SELECT Text FROM Note WHERE Id = ?1 AND DeletedAt IS NULL")?;
        let mut rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    /// Update an existing note's text. Caller must ensure the app is not running.
    pub fn update_text(&self, id: &str, plum_text: &str) -> Result<()> {
        backup::backup_if_needed(&self.path)?;
        let conn = self.conn_write()?;
        conn.execute(
            "UPDATE Note SET Text = ?1, UpdatedAt = ?2 WHERE Id = ?3",
            params![plum_text, ticks::now_ticks(), id],
        )?;
        Ok(())
    }

    /// Insert a new note by cloning field values from an existing healthy row.
    pub fn insert_note(&self, plum_text: &str, theme: &str) -> Result<String> {
        backup::backup_if_needed(&self.path)?;
        let conn = self.conn_write()?;
        let new_id = uuid::Uuid::new_v4().to_string();
        let now = ticks::now_ticks();

        // Clone all fields from an existing healthy row (Type/ParentId/ChangeKey semantics
        // are undocumented; copying a known-good row maximizes the chance the UWP app
        // accepts and syncs the new note).
        let mut stmt = conn.prepare(
            "SELECT WindowPosition, IsAlwaysOnTop, CreationNoteIdAnchor, IsFutureNote, \
             ChangeKey, RemoteSchemaVersion, IsRemoteDataInvalid, PendingInsightsScan, Type, ParentId \
             FROM Note LIMIT 1",
        )?;
        let template: Option<(Option<String>, i64, Option<String>, i64, Option<String>, i64, i64, i64, Option<String>, Option<String>)> = stmt
            .query_row([], |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                ))
            })
            .ok();
        let (window_pos, always_on_top, anchor, future, change_key, schema_ver, remote_invalid, _, ty, parent) =
            template.unwrap_or((None, 0, None, 0, None, 0, 0, 0, None, None));

        conn.execute(
            "INSERT INTO Note (Text, WindowPosition, IsOpen, IsAlwaysOnTop, CreationNoteIdAnchor, \
             Theme, IsFutureNote, RemoteId, ChangeKey, RemoteSchemaVersion, IsRemoteDataInvalid, \
             PendingInsightsScan, Type, Id, ParentId, CreatedAt, DeletedAt, UpdatedAt) \
             VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, 1, ?10, ?11, ?12, ?13, NULL, ?13)",
            params![
                plum_text, window_pos, always_on_top, anchor, theme, future,
                change_key, schema_ver, remote_invalid, ty, new_id, parent, now
            ],
        )?;
        Ok(new_id)
    }

    /// Soft-delete a note (moves it to the built-in app's trash).
    pub fn soft_delete(&self, id: &str) -> Result<()> {
        backup::backup_if_needed(&self.path)?;
        let conn = self.conn_write()?;
        conn.execute(
            "UPDATE Note SET DeletedAt = ?1, UpdatedAt = ?1 WHERE Id = ?2",
            params![ticks::now_ticks(), id],
        )?;
        Ok(())
    }

    /// Set a note's color theme.
    pub fn set_theme(&self, id: &str, theme: &str) -> Result<()> {
        backup::backup_if_needed(&self.path)?;
        let conn = self.conn_write()?;
        conn.execute("UPDATE Note SET Theme = ?1 WHERE Id = ?2", params![theme, id])?;
        Ok(())
    }
}

/// Strip Markdown formatting syntax from a single line of text so list previews show clean plain text.
pub fn strip_markdown_syntax(input: &str) -> String {
    let mut s = input.trim();

    // Strip leading block markers: headings (#), blockquotes (>), bullet markers (- / * / +)
    loop {
        if let Some(rest) = s.strip_prefix('#') {
            s = rest.trim_start();
        } else if let Some(rest) = s.strip_prefix('>') {
            s = rest.trim_start();
        } else if let Some(rest) = s
            .strip_prefix("- [ ] ")
            .or_else(|| s.strip_prefix("- [x] "))
            .or_else(|| s.strip_prefix("- [X] "))
            .or_else(|| s.strip_prefix("- "))
            .or_else(|| s.strip_prefix("* "))
            .or_else(|| s.strip_prefix("+ "))
        {
            s = rest.trim_start();
        } else {
            break;
        }
    }

    // Replace Markdown links `[label](url)` and images `![alt](url)` with their visible text,
    // and strip inline emphasis/code markers (`**`, `__`, `~~`, `` ` ``).
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        // Image `![alt](url)` -> `alt`
        if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some((label, next_idx)) = parse_md_link(&chars, i + 1) {
                out.push_str(&label);
                i = next_idx;
                continue;
            }
        }
        // Link `[label](url)` -> `label`
        if chars[i] == '[' {
            if let Some((label, next_idx)) = parse_md_link(&chars, i) {
                out.push_str(&label);
                i = next_idx;
                continue;
            }
        }
        // Strip `**`, `__`, `~~`
        if (chars[i] == '*' || chars[i] == '_' || chars[i] == '~')
            && chars.get(i + 1) == Some(&chars[i])
        {
            i += 2;
            continue;
        }
        // Strip inline code backticks
        if chars[i] == '`' {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }

    // Collapse consecutive whitespace into a single space
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_md_link(chars: &[char], start_bracket: usize) -> Option<(String, usize)> {
    debug_assert_eq!(chars[start_bracket], '[');
    let mut close_bracket = None;
    let mut j = start_bracket + 1;
    while j < chars.len() {
        if chars[j] == ']' {
            close_bracket = Some(j);
            break;
        }
        j += 1;
    }
    let close_b = close_bracket?;
    if chars.get(close_b + 1) != Some(&'(') {
        return None;
    }
    let mut k = close_b + 2;
    let mut depth = 1usize;
    while k < chars.len() {
        if chars[k] == '(' {
            depth += 1;
        } else if chars[k] == ')' {
            depth -= 1;
            if depth == 0 {
                let label: String = chars[start_bracket + 1..close_b].iter().collect();
                return Some((label, k + 1));
            }
        }
        k += 1;
    }
    None
}

/// Extract a display title and a brief content preview snippet.
pub fn extract_title_and_snippet(plum_text: &str) -> (String, String) {
    let paras = crate::markup::parse_plum(plum_text);
    let mut title = None;
    let mut snippet_parts: Vec<String> = Vec::new();

    for p in &paras {
        let raw_text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
        let cleaned = strip_markdown_syntax(&raw_text);
        let t = cleaned.trim();
        if t.is_empty() {
            continue;
        }
        if title.is_none() {
            let char_count = t.chars().count();
            if paras.len() <= 1 && char_count > 26 {
                let chars: Vec<char> = t.chars().collect();
                title = Some(chars[..26.min(chars.len())].iter().collect::<String>());
                snippet_parts.push(chars[26..].iter().collect::<String>());
            } else {
                title = Some(t.chars().take(30).collect::<String>());
            }
        } else {
            snippet_parts.push(t.to_string());
        }
    }

    let title_str = title.unwrap_or_else(|| "无标题便签".into());
    let joined = snippet_parts.join(" ");
    let snippet_str = if joined.is_empty() {
        String::new()
    } else {
        joined.chars().take(80).collect()
    };

    (title_str, snippet_str)
}
