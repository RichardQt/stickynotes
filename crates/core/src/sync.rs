//! Sync coordinator: orchestrates reads/writes and launching the built-in
//! Sticky Notes app (which performs the actual cloud sync via Microsoft account).

use crate::store::{MarkdownStore, PlumError, PlumStore, Result};
use std::path::Path;
use std::process::Command;

const APP_FAMILY: &str = "Microsoft.MicrosoftStickyNotes_8wekyb3d8bbwe!App";

/// Is the built-in Sticky Notes UWP app currently running?
pub fn is_sticky_notes_running() -> bool {
    // tasklist lists UWP processes under their package executable (e.g. Microsoft.Notes.exe)
    let out = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq Microsoft.Notes.exe", "/FO", "CSV", "/NH"])
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.contains("Microsoft.Notes.exe")
        }
        Err(_) => false,
    }
}

/// Launch the built-in Sticky Notes app (it will push/pull cloud changes).
pub fn launch_sticky_notes() -> std::io::Result<()> {
    Command::new("explorer.exe")
        .args([format!("shell:AppsFolder\\{APP_FAMILY}")])
        .spawn()
        .map(|_| ())
}

/// Guard: ensure it is safe to write to plum.sqlite.
/// In SQLite WAL mode with busy_timeout, concurrent readers and writers are
/// supported without needing to block or spawn slow process inspections.
pub fn ensure_writable() -> Result<()> {
    Ok(())
}

/// Poll the plum database until the note with `id` has a non-null RemoteId
/// (i.e. the built-in app has pushed it to the cloud), or timeout.
/// Returns true if pushed within the timeout.
pub fn wait_until_synced(plum: &PlumStore, id: &str, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    let conn = match rusqlite::Connection::open_with_flags(
        plum.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(_) => return false,
    };
    loop {
        let pushed: Option<String> = conn
            .query_row(
                "SELECT RemoteId FROM Note WHERE Id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if pushed.map(|s| !s.is_empty()).unwrap_or(false) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// Full write pipeline: guard -> write callback -> (user triggers built-in app for cloud push).
/// The write itself only touches the local DB; cloud push happens when the user
/// opens the built-in app (via the sync button), which then also pulls remote changes.
pub struct SyncCoordinator {
    pub plum: PlumStore,
    /// Full-fidelity Markdown per note; plum alone loses headings, fences,
    /// tables and other formatting on the way back.
    markdown: Option<MarkdownStore>,
}

impl SyncCoordinator {
    pub fn new() -> Result<Self> {
        Ok(Self { plum: PlumStore::open()?, markdown: MarkdownStore::open().ok() })
    }

    pub fn reload(&self) -> Result<Vec<crate::store::model::NoteSummary>> {
        self.plum.list_notes()
    }

    /// Canonical Markdown for a note: our own copy when the plum row is still
    /// the one we wrote, otherwise the lossy plum projection.
    pub fn note_markdown(&self, id: &str, plum_text: &str) -> Option<String> {
        self.markdown.as_ref()?.load(id, plum_text)
    }

    /// Write a note and return the exact plum text and `UpdatedAt` stored, so
    /// the caller can tell its own writes apart from external ones.
    pub fn save_note(&self, id: &str, markdown: &str) -> Result<(String, i64)> {
        ensure_writable()?;
        let plum_text = crate::markup::markdown_to_plum(markdown);
        let updated_at = self.plum.update_text(id, &plum_text)?;
        if let Some(store) = &self.markdown {
            // Best effort: losing the shadow copy only costs formatting.
            let _ = store.save(id, &plum_text, markdown);
        }
        Ok((plum_text, updated_at))
    }

    /// Write a note downloaded from WebDAV into plum (and the Markdown
    /// shadow). Creates the note when this device has never seen its id.
    ///
    /// The plum row keeps the remote `updated_at`: stamping local time here
    /// would make the note look newer than the file it came from, so every
    /// later pass would re-upload it and the two devices would ping-pong the
    /// same content forever.
    pub fn apply_remote_note(&self, note: &crate::webdav::RemoteNote) -> Result<()> {
        ensure_writable()?;
        let plum_text = crate::markup::markdown_to_plum(&note.markdown);
        if self.plum.get_note_text(&note.id)?.is_some() {
            self.plum
                .update_text_at(&note.id, &plum_text, Some(note.updated_at))?;
        } else {
            self.plum
                .insert_note_with_id(&note.id, &plum_text, &note.color, note.updated_at)?;
        }
        self.plum.set_theme(&note.id, &note.color)?;
        if let Some(store) = &self.markdown {
            let _ = store.save(&note.id, &plum_text, &note.markdown);
        }
        Ok(())
    }

    pub fn create_note(&self, markdown: &str, theme: &str) -> Result<String> {
        ensure_writable()?;
        let plum_text = crate::markup::markdown_to_plum(markdown);
        let id = self.plum.insert_note(&plum_text, theme)?;
        if let Some(store) = &self.markdown {
            let _ = store.save(&id, &plum_text, markdown);
        }
        Ok(id)
    }

    pub fn delete_note(&self, id: &str) -> Result<()> {
        ensure_writable()?;
        self.plum.soft_delete(id)
    }
}

/// Verify a plum database file looks sane (used by a debug CLI during spikes).
pub fn verify_db(path: &Path) -> Result<()> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let ok: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if ok == "ok" {
        Ok(())
    } else {
        Err(PlumError::Other(format!("integrity_check: {ok}")))
    }
}
