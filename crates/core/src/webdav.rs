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
use std::collections::HashSet;
use std::time::Duration;

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:resourcetype/><d:getlastmodified/></d:prop>
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
    pub unchanged: usize,
    /// Remote versions newer than the local copy; applied by the caller.
    pub remote_notes: Vec<RemoteNote>,
}

impl SyncReport {
    pub fn summary(&self) -> String {
        format!(
            "同步完成：上传 {} · 下载 {} · 未变 {}",
            self.uploaded, self.downloaded, self.unchanged
        )
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

/// One full sync pass. Network work only: local writes are left to the caller.
pub fn sync(cfg: &WebDavConfig, notes: &[SyncNote]) -> Result<SyncReport, String> {
    let client = WebDavClient::new(cfg)?;
    client.ensure_dir(&cfg.dir)?;
    let files = client.list(&cfg.dir)?;
    let remote: HashSet<&str> = files.iter().map(String::as_str).collect();

    let mut report = SyncReport::default();
    let mut local_ids: HashSet<&str> = HashSet::new();

    for note in notes {
        local_ids.insert(note.id.as_str());
        let file = format!("{}.md", note.id);
        if !remote.contains(file.as_str()) {
            client.put(&cfg.dir, &file, &render_markdown_file(note))?;
            report.uploaded += 1;
            continue;
        }
        let text = client.get(&cfg.dir, &file)?;
        let Some((meta, body)) = parse_markdown_file(&text) else {
            // Not one of our files: leave it alone.
            report.unchanged += 1;
            continue;
        };
        if meta.updated_at > note.updated_at {
            report.remote_notes.push(meta.into_remote(body));
            report.downloaded += 1;
        } else if note.updated_at > meta.updated_at {
            client.put(&cfg.dir, &file, &render_markdown_file(note))?;
            report.uploaded += 1;
        } else {
            report.unchanged += 1;
        }
    }

    // Notes that exist only on the server are created locally.
    for file in &files {
        let Some(id) = file.strip_suffix(".md") else { continue };
        if local_ids.contains(id) {
            continue;
        }
        let text = client.get(&cfg.dir, file)?;
        if let Some((meta, body)) = parse_markdown_file(&text) {
            report.remote_notes.push(meta.into_remote(body));
            report.downloaded += 1;
        }
    }

    Ok(report)
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

    /// `.md` file names directly inside `dir`.
    pub fn list(&self, dir: &str) -> Result<Vec<String>, String> {
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
        Ok(parse_propfind_hrefs(&xml)
            .into_iter()
            .filter_map(|href| file_name_of(&href))
            .filter(|name| name.ends_with(".md"))
            .collect())
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

    pub fn put(&self, dir: &str, name: &str, content: &str) -> Result<(), String> {
        let resp = self
            .request(Method::PUT, &self.url(&join_path(dir, name)))
            .header("Content-Type", "text/markdown; charset=utf-8")
            .body(content.to_string())
            .send()
            .map_err(|e| format!("上传 {name} 失败：{e}"))?;
        check(resp, &format!("上传 {name}"))
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

/// Extract every `<D:href>` value from a Multi-Status response.
fn parse_propfind_hrefs(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with('/') {
            continue;
        }
        let local = tag
            .split(|c: char| c == ':' || c.is_whitespace())
            .last()
            .unwrap_or("")
            .trim_end_matches('/');
        if !local.eq_ignore_ascii_case("href") {
            continue;
        }
        // Text until the closing tag.
        let Some(end) = rest.find('<') else { break };
        let value = decode_xml_entities(rest[..end].trim());
        out.push(value);
        rest = &rest[end..];
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
    fn propfind_hrefs_are_extracted_and_decoded() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response><d:href>/dav/StickyNotes/</d:href><d:propstat>
    <d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
  </d:propstat></d:response>
  <d:response><d:href>/dav/StickyNotes/abc%201.md</d:href></d:response>
</d:multistatus>"#;
        let hrefs = parse_propfind_hrefs(xml);
        assert_eq!(hrefs.len(), 2);
        assert_eq!(file_name_of(&hrefs[0]).as_deref(), Some("StickyNotes"));
        assert_eq!(file_name_of(&hrefs[1]).as_deref(), Some("abc 1.md"));
    }

    #[test]
    fn joins_remote_paths() {
        assert_eq!(join_path("/StickyNotes/", "a.md"), "/StickyNotes/a.md");
        assert_eq!(join_path("StickyNotes", "a.md"), "/StickyNotes/a.md");
        assert_eq!(join_path("", "a.md"), "/a.md");
    }
}
