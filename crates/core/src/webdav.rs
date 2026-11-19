//! WebDAV client and bidirectional Markdown sync.
//!
//! Every note lives remotely as `<note_id>.md` with a small YAML frontmatter
//! block (id, title, color, tags, pinned, updated_at) followed by the note body
//! as plain Markdown, so the files stay readable in any Markdown editor.
//!
//! Conflicts are resolved by `updated_at` (latest wins); the sync touches only
//! files whose content actually differs.

use crate::config::WebDavConfig;
use crate::ticks;
use reqwest::blocking::{Client, Response};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype/>
    <d:getlastmodified/>
    <d:getetag/>
    <d:getcontentlength/>
  </d:prop>
</d:propfind>"#;

#[derive(Debug, Clone)]
pub struct SyncNote {
    pub id: String,
    pub title: String,
    pub color: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    /// .NET ticks, same unit as the plum `UpdatedAt` column.
    pub updated_at: i64,
    /// Full Markdown (title line + body).
    pub markdown: String,
}

#[derive(Debug, Clone)]
pub struct RemoteNote {
    pub id: String,
    pub title: String,
    pub color: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub updated_at: i64,
    /// Full Markdown (title line + body).
    pub markdown: String,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub uploaded: usize,
    pub downloaded: usize,
    /// Remote `.md` files deleted because the note was deleted locally.
    pub deleted_remote: usize,
    /// Local notes whose file vanished from the server while this device had
    /// no newer edit — the deletion wins here too; applied by the caller.
    pub deleted_local: Vec<String>,
    pub unchanged: usize,
    /// Remote versions newer than the local copy; applied by the caller.
    pub remote_notes: Vec<RemoteNote>,
}

impl SyncReport {
    pub fn summary(&self) -> String {
        let deleted = self.deleted_remote + self.deleted_local.len();
        format!(
            "同步完成：上传 {} · 下载 {} · 删除 {} · 未变 {}",
            self.uploaded, self.downloaded, deleted, self.unchanged
        )
    }
}

/// One file as reported by PROPFIND Depth: 1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteEntry {
    pub name: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub size: Option<u64>,
}

/// What the server looked like the last time a file was read or written.
/// Only version markers and the note timestamp — never titles or content.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedFile {
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// The note's `updated_at` parsed from the file, so an unchanged remote
    /// file can be compared without downloading it again.
    #[serde(default)]
    pub updated_at: i64,
}

impl CachedFile {
    fn from_entry(entry: &RemoteEntry, updated_at: i64) -> Self {
        Self {
            etag: entry.etag.clone(),
            last_modified: entry.last_modified.clone(),
            size: entry.size,
            updated_at,
        }
    }
}

/// Version snapshot of the remote collection, persisted across restarts so
/// the first sync after launch is cheap too.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncCache {
    /// Server base URL this cache belongs to.
    #[serde(default)]
    pub url: String,
    /// Remote collection, normalised (`/StickyNotes`).
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub files: BTreeMap<String, CachedFile>,
}

pub fn cache_path() -> PathBuf {
    crate::config::config_dir().join("webdav-cache.json")
}

impl SyncCache {
    pub fn load() -> Self {
        std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = crate::config::config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(cache_path(), text)
    }

    /// Is this cache still about the configured server and directory?
    pub fn matches(&self, cfg: &WebDavConfig) -> bool {
        self.url == cache_url(cfg) && self.dir == normalized_dir(&cfg.dir)
    }
}

/// Result of one sync pass: the report plus the refreshed version cache and
/// the pruned deletion ledger (both to be persisted by the caller).
#[derive(Debug)]
pub struct SyncOutcome {
    pub report: SyncReport,
    pub cache: SyncCache,
    pub ledger: DeleteLedger,
}

/// Deletion ledger: note ids deleted on this device whose remote `.md` has
/// not been removed yet. Survives restarts, so an unlink that happened while
/// WebDAV was off (or down) still propagates on the next pass. Only ids and
/// timestamps — never note content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteLedger {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub dir: String,
    /// note id -> .NET ticks when the local delete happened.
    #[serde(default)]
    pub deleted: BTreeMap<String, i64>,
}

pub fn ledger_path() -> PathBuf {
    crate::config::config_dir().join("webdav-deletions.json")
}

impl Default for DeleteLedger {
    fn default() -> Self {
        Self { url: String::new(), dir: String::new(), deleted: BTreeMap::new() }
    }
}

impl DeleteLedger {
    pub fn load() -> Self {
        std::fs::read_to_string(ledger_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = crate::config::config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(ledger_path(), text)
    }

    pub fn record(&mut self, cfg: &WebDavConfig, id: &str, deleted_at: i64) {
        self.url = cache_url(cfg);
        self.dir = normalized_dir(&cfg.dir);
        self.deleted.insert(id.to_string(), deleted_at);
    }

    /// Belongs to the same server and collection? A different target means
    /// the old ledger's deletions say nothing about this server's files.
    pub fn matches(&self, cfg: &WebDavConfig) -> bool {
        self.url == cache_url(cfg) && self.dir == normalized_dir(&cfg.dir)
    }

    /// Drop ids that no longer need remote deletion. `remote_gone`: ids whose
    /// file is no longer on the server; `revived`: ids that exist locally
    /// again (re-created on another device and pulled here — a later deletion
    /// must win, the ledger only tracks the newest intent).
    pub fn prune(
        &mut self,
        remote_gone: &BTreeMap<String, i64>,
        revived: &BTreeSet<String>,
    ) {
        self.deleted
            .retain(|id, _| !remote_gone.contains_key(id) && !revived.contains(id));
    }
}

// ---------------------------------------------------------------------------
// Markdown files with YAML frontmatter
// ---------------------------------------------------------------------------

fn iso8601(ticks: i64) -> String {
    let (secs, nanos) = ticks::ticks_to_unix(ticks);
    time::OffsetDateTime::from_unix_timestamp_nanos(
        secs as i128 * 1_000_000_000 + nanos as i128,
    )
    .ok()
    .and_then(|dt| dt.format(&time::format_description::well_known::Rfc3339).ok())
    .unwrap_or_default()
}

fn parse_iso(value: &str) -> Option<i64> {
    let dt = time::OffsetDateTime::parse(
        value,
        &time::format_description::well_known::Rfc3339,
    )
    .ok()?;
    let nanos = dt.unix_timestamp_nanos();
    let secs = (nanos / 1_000_000_000) as i64;
    let sub = (nanos % 1_000_000_000) as u32;
    Some(ticks::unix_to_ticks(secs, sub))
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

fn unquoted(value: &str) -> String {
    let v = value.trim();
    if v.starts_with('"') {
        serde_json::from_str(v).unwrap_or_else(|_| v.trim_matches('"').to_string())
    } else {
        v.to_string()
    }
}

/// Render one note the way it is stored on the server.
pub fn render_markdown_file(note: &SyncNote) -> String {
    let body = body_without_title(&note.markdown, &note.title);
    let color = if note.color.trim().is_empty() { "Yellow" } else { note.color.as_str() };
    let tags = serde_json::to_string(&note.tags).unwrap_or_else(|_| "[]".into());
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("id: {}\n", note.id));
    out.push_str(&format!("title: {}\n", quoted(&note.title)));
    out.push_str(&format!("color: {color}\n"));
    out.push_str(&format!("tags: {tags}\n"));
    out.push_str(&format!("pinned: {}\n", note.pinned));
    out.push_str(&format!("updated_at: {}\n", iso8601(note.updated_at)));
    out.push_str("---\n\n");
    out.push_str(body.trim_end());
    out.push('\n');
    out
}

#[derive(Debug, Default)]
struct Meta {
    id: String,
    title: String,
    color: String,
    tags: Vec<String>,
    pinned: bool,
    updated_at: i64,
}

fn parse_markdown_file(text: &str) -> Option<(Meta, String)> {
    let text = text.replace("\r\n", "\n");
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let after = text.strip_prefix("---\n")?;
    let end = after.find("\n---")?;
    let head = &after[..end];
    let body = after[end + 4..].strip_prefix('\n').unwrap_or(&after[end + 4..]);

    let mut meta = Meta { color: "Yellow".into(), ..Default::default() };
    for line in head.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match key.trim() {
            "id" => meta.id = unquoted(value),
            "title" => meta.title = unquoted(value),
            "color" => {
                let c = unquoted(value);
                if !c.is_empty() {
                    meta.color = c;
                }
            }
            "tags" => {
                meta.tags = if value.starts_with('[') {
                    serde_json::from_str(value).unwrap_or_default()
                } else {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                };
            }
            "pinned" => meta.pinned = value.eq_ignore_ascii_case("true"),
            "updated_at" => meta.updated_at = parse_iso(value).unwrap_or(0),
            _ => {}
        }
    }
    if meta.id.is_empty() {
        return None;
    }
    Some((meta, body.to_string()))
}

impl Meta {
    fn into_remote(self, body: String) -> RemoteNote {
        let body = body.trim_end();
        let markdown = if body.is_empty() {
            self.title.clone()
        } else if self.title.is_empty() {
            body.to_string()
        } else {
            format!("{}\n{}", self.title, body)
        };
        RemoteNote {
            id: self.id,
            title: self.title,
            color: self.color,
            tags: self.tags,
            pinned: self.pinned,
            updated_at: self.updated_at,
            markdown,
        }
    }
}

/// The note body without its title line, for the frontmatter + body layout.
fn body_without_title(markdown: &str, title: &str) -> String {
    let md = markdown.replace("\r\n", "\n");
    let (first, rest) = match md.split_once('\n') {
        Some((first, rest)) => (first, rest),
        None => (md.as_str(), ""),
    };
    let title = title.trim();
    if title.is_empty()
        || first.trim() == title
        || crate::store::strip_markdown_syntax(first) == title
    {
        rest.trim_start_matches('\n').to_string()
    } else {
        md
    }
}

// ---------------------------------------------------------------------------
// WebDAV client
// ---------------------------------------------------------------------------

pub struct WebDavClient {
    client: Client,
    base: String,
    username: String,
    password: String,
}

pub fn test_connection(cfg: &WebDavConfig) -> Result<String, String> {
    let client = WebDavClient::new(cfg)?;
    client.test()
}

/// One sync pass against the server. Network work only: local writes are left
/// to the caller.
///
/// `cache` holds the ETag / Last-Modified of every file seen during the last
/// pass, so unchanged remote files are never downloaded again. Pass an empty
/// cache to force a full comparison. `ensure_dir` should be false once the
/// collection is known to exist during this run.
///
/// `ledger` carries note ids deleted locally whose remote file must go. A
/// deleted note's remote `.md` would otherwise come straight back as a
/// "remote-only" note on the next pass. Ids whose file is gone from the
/// server and ids that exist locally again are handed back so the caller can
/// persist the pruned ledger.
pub fn sync(
    cfg: &WebDavConfig,
    notes: &[SyncNote],
    cache: &SyncCache,
    ensure_dir: bool,
    ledger: &mut DeleteLedger,
) -> Result<SyncOutcome, String> {
    let client = WebDavClient::new(cfg)?;
    if ensure_dir {
        client.ensure_dir(&cfg.dir)?;
    }
    let entries = client.list_detailed(&cfg.dir)?;
    let remote_ids: BTreeMap<&str, &RemoteEntry> = entries
        .iter()
        .filter_map(|e| e.name.strip_suffix(".md").map(|id| (id, e)))
        .collect();

    let mut report = SyncReport::default();
    let mut fresh = SyncCache {
        url: cache_url(cfg),
        dir: normalized_dir(&cfg.dir),
        files: BTreeMap::new(),
    };

    // Pending local deletions first: those files must not be treated as live
    // remote notes below.
    let mut remote_deleted: BTreeMap<String, i64> = BTreeMap::new();
    let mut revived: BTreeSet<String> = BTreeSet::new();
    for id in ledger.deleted.keys().cloned().collect::<Vec<String>>() {
        if notes.iter().any(|n| n.id == id) {
            // Exists locally again (un-deleted here, or re-created on another
            // device and pulled in since): the deletion is no longer the
            // newest intent for this id.
            revived.insert(id);
            continue;
        }
        let file = format!("{id}.md");
        client.delete(&cfg.dir, &file)?;
        report.deleted_remote += 1;
        let deleted_at = ledger.deleted[&id];
        remote_deleted.insert(id, deleted_at);
        fresh.files.remove(&file);
    }
    ledger.prune(&remote_deleted, &revived);

    // Ids whose file was just removed must not be re-imported below.
    let just_deleted: BTreeSet<&str> = remote_deleted.keys().map(String::as_str).collect();

    for entry in &entries {
        let Some(id) = entry.name.strip_suffix(".md") else { continue };
        if just_deleted.contains(id) {
            continue;
        }
        let local = notes.iter().find(|n| n.id == id);
        let hit = cache
            .files
            .get(&entry.name)
            .filter(|c| same_version(c, entry));

        // `Known` means the cached versions still match, so the file was not
        // touched since the last pass and its body is not needed.
        let remote = match hit {
            Some(c) => RemoteVersion::Known { updated_at: c.updated_at },
            None => match fetch_remote(&client, &cfg.dir, &entry.name)? {
                Some((meta, body)) => RemoteVersion::Fetched { meta, body },
                None => {
                    // Not one of our files (or truncated): leave it alone.
                    report.unchanged += 1;
                    continue;
                }
            },
        };
        let remote_updated = match &remote {
            RemoteVersion::Known { updated_at } => *updated_at,
            RemoteVersion::Fetched { meta, .. } => meta.updated_at,
        };
        fresh
            .files
            .insert(entry.name.clone(), CachedFile::from_entry(entry, remote_updated));

        let Some(local) = local else {
            // Remote-only file: needs its body before it can be created here.
            let Some((meta, body)) = into_body(&client, &cfg.dir, &entry.name, remote)? else {
                continue;
            };
            fresh
                .files
                .insert(entry.name.clone(), CachedFile::from_entry(entry, meta.updated_at));
            report.remote_notes.push(meta.into_remote(body));
            report.downloaded += 1;
            continue;
        };

        if remote_updated > local.updated_at {
            let Some((meta, body)) = into_body(&client, &cfg.dir, &entry.name, remote)? else {
                continue;
            };
            fresh
                .files
                .insert(entry.name.clone(), CachedFile::from_entry(entry, meta.updated_at));
            report.remote_notes.push(meta.into_remote(body));
            report.downloaded += 1;
        } else if local.updated_at > remote_updated {
            let new_etag =
                client.put(&cfg.dir, &entry.name, &render_markdown_file(local))?;
            report.uploaded += 1;
            // The server replaced the file; remember the ETag it returned (if
            // any) so the next pass does not have to download our own upload.
            fresh.files.insert(
                entry.name.clone(),
                CachedFile {
                    etag: new_etag,
                    last_modified: None,
                    size: None,
                    updated_at: local.updated_at,
                },
            );
        } else {
            report.unchanged += 1;
        }
    }

    // Notes that exist only locally.
    for note in notes {
        let id = note.id.as_str();
        let file = format!("{id}.md");
        if just_deleted.contains(id) {
            continue;
        }
        // Present on the server (handled in the entries loop above)?
        if remote_ids.contains_key(id) {
            continue;
        }
        match cache.files.get(&file) {
            // We knew this file; it vanished from the server. If this device
            // has not touched the note since (local time == the version we
            // last saw), the deletion was made on another device and wins
            // here too. A newer local edit means this device was working
            // offline: upload ours, the timestamps decide on other devices.
            Some(c) if note.updated_at <= c.updated_at => {
                report.deleted_local.push(id.to_string());
            }
            _ => {
                let new_etag = client.put(&cfg.dir, &file, &render_markdown_file(note))?;
                report.uploaded += 1;
                fresh.files.insert(
                    file,
                    CachedFile {
                        etag: new_etag,
                        last_modified: None,
                        size: None,
                        updated_at: note.updated_at,
                    },
                );
            }
        }
    }

    Ok(SyncOutcome { report, cache: fresh, ledger: ledger.clone() })
}

enum RemoteVersion {
    /// Version markers matched the cache; body not downloaded.
    Known { updated_at: i64 },
    Fetched { meta: Meta, body: String },
}

fn fetch_remote(
    client: &WebDavClient,
    dir: &str,
    name: &str,
) -> Result<Option<(Meta, String)>, String> {
    let text = client.get(dir, name)?;
    Ok(parse_markdown_file(&text))
}

/// Fetch a body unless this pass already has it.
fn into_body(
    client: &WebDavClient,
    dir: &str,
    name: &str,
    remote: RemoteVersion,
) -> Result<Option<(Meta, String)>, String> {
    match remote {
        RemoteVersion::Fetched { meta, body } => Ok(Some((meta, body))),
        RemoteVersion::Known { .. } => fetch_remote(client, dir, name),
    }
}

/// Normalised ETag: some servers return `"abc"` from PROPFIND but `W/"abc"`
/// from a PUT response (or vice versa); the prefix and quoting are not part
/// of the identity.
fn canonical_etag(etag: &str) -> &str {
    let e = etag.trim();
    let e = e.strip_prefix("W/").unwrap_or(e);
    e.trim_matches('"')
}

/// Whether a PROPFIND entry still describes the file we cached.
///
/// ETag is authoritative when both sides provide one. Servers without ETags
/// fall back to Last-Modified + Content-Length; with neither pair we must
/// download to compare.
fn same_version(cached: &CachedFile, entry: &RemoteEntry) -> bool {
    if let (Some(a), Some(b)) = (cached.etag.as_deref(), entry.etag.as_deref()) {
        return canonical_etag(a) == canonical_etag(b);
    }
    match (
        cached.last_modified.as_deref(),
        entry.last_modified.as_deref(),
        cached.size,
        entry.size,
    ) {
        (Some(a), Some(b), Some(sa), Some(sb)) => a == b && sa == sb,
        (Some(a), Some(b), None, None) => a == b,
        _ => false,
    }
}

impl WebDavClient {
    pub fn new(cfg: &WebDavConfig) -> Result<Self, String> {
        let base = cfg.url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err("WebDAV 地址不能为空".into());
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("StickyNotes/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("无法创建 HTTP 客户端：{e}"))?;
        Ok(Self {
            client,
            base: base.to_string(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn request(&self, method: Method, url: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
    }

    /// PROPFIND Depth: 0 — validates reachability, TLS and credentials.
    pub fn test(&self) -> Result<String, String> {
        let method = Method::from_bytes(b"PROPFIND").map_err(|e| e.to_string())?;
        let resp = self
            .request(method, &format!("{}/", self.base))
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY)
            .send()
            .map_err(|e| format!("连接失败：{e}"))?;
        let status = resp.status();
        if status.is_success() {
            Ok(format!("连接成功（HTTP {}）", status.as_u16()))
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            Err("认证失败（HTTP 401），请检查用户名或密码".into())
        } else if status == reqwest::StatusCode::FORBIDDEN {
            Err("无访问权限（HTTP 403）".into())
        } else {
            Err(format!("服务器返回 HTTP {}", status.as_u16()))
        }
    }

    /// Create the collection and every missing parent (MKCOL), ignoring
    /// “already exists” responses.
    pub fn ensure_dir(&self, dir: &str) -> Result<(), String> {
        let method = Method::from_bytes(b"MKCOL").map_err(|e| e.to_string())?;
        let mut path = String::new();
        for segment in dir.split('/').filter(|s| !s.is_empty()) {
            path.push('/');
            path.push_str(segment);
            let resp = self
                .request(method.clone(), &self.url(&format!("{path}/")))
                .send()
                .map_err(|e| format!("创建目录失败：{e}"))?;
            let status = resp.status();
            // 405 = already exists; 301 = server wants the canonical slash URL.
            if !(status.is_success() || status.as_u16() == 405 || status.as_u16() == 301) {
                return Err(format!("创建目录 {path} 失败：HTTP {}", status.as_u16()));
            }
        }
        Ok(())
    }

    /// `.md` files directly inside `dir`, with their version markers.
    pub fn list_detailed(&self, dir: &str) -> Result<Vec<RemoteEntry>, String> {
        let method = Method::from_bytes(b"PROPFIND").map_err(|e| e.to_string())?;
        let dir_path = normalized_dir(dir);
        let collection = if dir_path == "/" {
            "/".to_string()
        } else {
            format!("{dir_path}/")
        };
        let resp = self
            .request(method, &self.url(&collection))
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY)
            .send()
            .map_err(|e| format!("获取文件列表失败：{e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("获取文件列表失败：HTTP {}", status.as_u16()));
        }
        let xml = resp.text().map_err(|e| e.to_string())?;
        let mut entries: Vec<RemoteEntry> = parse_propfind_entries(&xml)
            .into_iter()
            .filter(|entry| entry.name.ends_with(".md"))
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    pub fn get(&self, dir: &str, name: &str) -> Result<String, String> {
        let resp = self
            .request(Method::GET, &self.url(&join_path(dir, name)))
            .send()
            .map_err(|e| format!("下载 {name} 失败：{e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("下载 {name} 失败：HTTP {}", status.as_u16()));
        }
        resp.text().map_err(|e| format!("读取 {name} 失败：{e}"))
    }

    /// Upload one file; returns the ETag the server assigned, when it sends one.
    pub fn put(&self, dir: &str, name: &str, content: &str) -> Result<Option<String>, String> {
        let resp = self
            .request(Method::PUT, &self.url(&join_path(dir, name)))
            .header("Content-Type", "text/markdown; charset=utf-8")
            .body(content.to_string())
            .send()
            .map_err(|e| format!("上传 {name} 失败：{e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("上传 {name} 失败：HTTP {}", status.as_u16()));
        }
        Ok(resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string))
    }

    pub fn delete(&self, dir: &str, name: &str) -> Result<(), String> {
        let resp = self
            .request(Method::DELETE, &self.url(&join_path(dir, name)))
            .send()
            .map_err(|e| format!("删除 {name} 失败：{e}"))?;
        check(resp, &format!("删除 {name}"))
    }
}

fn check(resp: Response, what: &str) -> Result<(), String> {
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{what} 失败：HTTP {}", status.as_u16()))
    }
}

fn cache_url(cfg: &WebDavConfig) -> String {
    cfg.url.trim().trim_end_matches('/').to_string()
}

/// `/StickyNotes/`, `StickyNotes` and `""` all normalise to `/StickyNotes`.
fn normalized_dir(dir: &str) -> String {
    let segments = dir.trim().trim_matches('/');
    if segments.is_empty() { "/".into() } else { format!("/{segments}") }
}

fn join_path(dir: &str, name: &str) -> String {
    let dir = normalized_dir(dir);
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn file_name_of(href: &str) -> Option<String> {
    let href = href.split(['?', '#']).next().unwrap_or(href);
    let last = href.trim_end_matches('/').rsplit('/').next()?;
    if last.is_empty() {
        return None;
    }
    Some(percent_decode(last))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Local name of an XML tag, independent of namespace prefix and casing.
fn local_tag_name(tag: &str) -> String {
    let name = tag.split_whitespace().next().unwrap_or("");
    let name = name.trim_end_matches('/');
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

/// Extract every `<response>` with its href, ETag, Last-Modified and size from
/// a Multi-Status response.
fn parse_propfind_entries(xml: &str) -> Vec<RemoteEntry> {
    let mut out = Vec::new();
    let mut current: Option<RemoteEntry> = None;
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let after = &rest[lt + 1..];
        let Some(gt) = after.find('>') else { break };
        let tag = &after[..gt];
        rest = &after[gt + 1..];
        // XML declaration, comment or doctype.
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let local = local_tag_name(tag);
        let text = if closing || self_closing {
            None
        } else {
            rest.find('<').map(|end| decode_xml_entities(rest[..end].trim()))
        };
        match local.as_str() {
            "response" => {
                if closing {
                    if let Some(entry) = current.take() {
                        if !entry.name.is_empty() {
                            out.push(entry);
                        }
                    }
                } else {
                    current = Some(RemoteEntry::default());
                }
            }
            "href" => {
                if let (Some(value), Some(entry)) = (text, current.as_mut()) {
                    if entry.name.is_empty() {
                        if let Some(name) = file_name_of(&value) {
                            entry.name = name;
                        }
                    }
                }
            }
            "getetag" => {
                if let (Some(value), Some(entry)) = (text, current.as_mut()) {
                    if !value.is_empty() {
                        entry.etag = Some(value);
                    }
                }
            }
            "getlastmodified" => {
                if let (Some(value), Some(entry)) = (text, current.as_mut()) {
                    if !value.is_empty() {
                        entry.last_modified = Some(value);
                    }
                }
            }
            "getcontentlength" => {
                if let (Some(value), Some(entry)) = (text, current.as_mut()) {
                    if let Ok(size) = value.parse::<u64>() {
                        entry.size = Some(size);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(entry) = current {
        if !entry.name.is_empty() {
            out.push(entry);
        }
    }
    out
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> SyncNote {
        SyncNote {
            id: "11111111-2222-3333-4444-555555555555".into(),
            title: "测试标题".into(),
            color: "Green".into(),
            tags: vec!["工作".into(), "dav".into()],
            pinned: true,
            updated_at: 639_000_000_000_000_000,
            markdown: "# 测试标题\n\n正文 **加粗** 内容\n```\ncode\n```".into(),
        }
    }

    #[test]
    fn markdown_file_roundtrip_keeps_metadata_and_body() {
        let rendered = render_markdown_file(&note());
        assert!(rendered.starts_with("---\n"));
        assert!(rendered.contains("title: \"测试标题\""));
        assert!(rendered.contains("tags: [\"工作\",\"dav\"]"));
        assert!(rendered.contains("pinned: true"));
        assert!(rendered.contains("\n---\n\n"));
        // The body does not repeat the title line.
        assert!(!rendered.contains("\n# 测试标题"));

        let (meta, body) = parse_markdown_file(&rendered).expect("parses");
        assert_eq!(meta.id, note().id);
        assert_eq!(meta.title, "测试标题");
        assert_eq!(meta.color, "Green");
        assert_eq!(meta.tags, vec!["工作".to_string(), "dav".into()]);
        assert!(meta.pinned);
        assert_eq!(meta.updated_at, note().updated_at);
        assert!(body.contains("```\ncode\n```"));

        let remote = meta.into_remote(body);
        assert_eq!(remote.markdown, "测试标题\n\n正文 **加粗** 内容\n```\ncode\n```");
    }

    #[test]
    fn plain_body_without_frontmatter_is_rejected() {
        assert!(parse_markdown_file("# Hello\nbody").is_none());
    }

    #[test]
    fn propfind_entries_carry_etag_modified_and_size() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response><d:href>/dav/StickyNotes/</d:href><d:propstat>
    <d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
  </d:propstat></d:response>
  <d:response>
    <d:href>/dav/StickyNotes/abc%201.md</d:href>
    <d:propstat><d:prop>
      <d:getlastmodified>Wed, 21 Oct 2015 07:28:00 GMT</d:getlastmodified>
      <d:getetag>"abc-1"</d:getetag>
      <d:getcontentlength>42</d:getcontentlength>
    </d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;
        let entries = parse_propfind_entries(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "StickyNotes");
        assert_eq!(entries[1].name, "abc 1.md");
        assert_eq!(entries[1].etag.as_deref(), Some("\"abc-1\""));
        assert_eq!(
            entries[1].last_modified.as_deref(),
            Some("Wed, 21 Oct 2015 07:28:00 GMT")
        );
        assert_eq!(entries[1].size, Some(42));
    }

    #[test]
    fn cached_versions_match_by_etag_or_by_mtime_and_size() {
        let entry = RemoteEntry {
            name: "a.md".into(),
            etag: Some("\"1\"".into()),
            last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".into()),
            size: Some(10),
        };
        let cached = CachedFile::from_entry(&entry, 5);
        assert!(same_version(&cached, &entry));

        let mut changed_etag = entry.clone();
        changed_etag.etag = Some("\"2\"".into());
        assert!(!same_version(&cached, &changed_etag));

        // Servers without ETags fall back to Last-Modified + Content-Length.
        let no_etag = RemoteEntry { etag: None, ..entry.clone() };
        let cached = CachedFile::from_entry(&no_etag, 5);
        assert!(same_version(&cached, &no_etag));
        let mut touched = no_etag.clone();
        touched.last_modified = Some("Thu, 22 Oct 2015 07:28:00 GMT".into());
        assert!(!same_version(&cached, &touched));
        let mut resized = no_etag.clone();
        resized.size = Some(11);
        assert!(!same_version(&cached, &resized));

        // Nothing to compare: must download to know.
        let bare = RemoteEntry { name: "b.md".into(), ..Default::default() };
        assert!(!same_version(&CachedFile::default(), &bare));
    }

    #[test]
    fn cache_serialises_and_tracks_its_collection() {
        let cfg = WebDavConfig {
            url: "https://example.com/dav/".into(),
            dir: "StickyNotes".into(),
            ..Default::default()
        };
        let mut cache = SyncCache::default();
        assert!(!cache.matches(&cfg));
        cache.url = "https://example.com/dav".into();
        cache.dir = "/StickyNotes".into();
        assert!(cache.matches(&cfg));
        cache.files.insert(
            "a.md".into(),
            CachedFile {
                etag: Some("\"1\"".into()),
                last_modified: None,
                size: Some(3),
                updated_at: 7,
            },
        );

        let text = serde_json::to_string(&cache).unwrap();
        let back: SyncCache = serde_json::from_str(&text).unwrap();
        assert!(back.matches(&cfg));
        assert_eq!(back.files["a.md"].updated_at, 7);
        assert_eq!(back.files["a.md"].etag.as_deref(), Some("\"1\""));

        let other = WebDavConfig {
            url: "https://example.com/dav".into(),
            dir: "/Other".into(),
            ..Default::default()
        };
        assert!(!back.matches(&other));
    }

    #[test]
    fn ledger_records_and_prunes_tombstones() {
        let cfg = WebDavConfig {
            url: "https://example.com/dav".into(),
            dir: "/StickyNotes".into(),
            ..Default::default()
        };
        let mut ledger = DeleteLedger::default();
        ledger.record(&cfg, "11111111-1111-1111-1111-111111111111", 100);
        ledger.record(&cfg, "22222222-2222-2222-2222-222222222222", 200);
        assert!(ledger.matches(&cfg));

        let mut gone = std::collections::BTreeMap::new();
        gone.insert("11111111-1111-1111-1111-111111111111".to_string(), 100);
        let mut revived = std::collections::BTreeSet::new();
        revived.insert("22222222-2222-2222-2222-222222222222".to_string());
        ledger.prune(&gone, &revived);
        assert!(ledger.deleted.is_empty());

        // A ledger from a different server/directory must be discarded.
        let other = WebDavConfig {
            url: "https://other.example.com/dav".into(),
            dir: "/StickyNotes".into(),
            ..Default::default()
        };
        assert!(!ledger.matches(&other));
    }

    #[test]
    fn weak_etag_prefixes_compare_equal() {
        assert_eq!(canonical_etag("W/\"1\""), canonical_etag("\"1\""));
        assert_eq!(canonical_etag("\"1\""), canonical_etag("\"1\""));
        assert_ne!(canonical_etag("\"1\""), canonical_etag("\"2\""));
    }

    #[test]
    fn joins_remote_paths() {
        assert_eq!(join_path("/StickyNotes/", "a.md"), "/StickyNotes/a.md");
        assert_eq!(join_path("StickyNotes", "a.md"), "/StickyNotes/a.md");
        assert_eq!(join_path("", "a.md"), "/a.md");
    }
}
