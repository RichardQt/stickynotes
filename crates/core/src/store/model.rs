//! Data model of the note database.

/// One row of the note list: enough to draw a card without the full text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSummary {
    pub id: String,
    /// First non-empty line, markup stripped (for the list view).
    pub title: String,
    /// Brief snippet of subsequent text for preview.
    pub snippet: String,
    /// Paper colour name (`Yellow`, `Blue`, …).
    pub theme: String,
    /// .NET ticks (UTC), like every timestamp in the database.
    pub created_at: i64,
    pub updated_at: i64,
}

/// A live note with its full content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    /// Canonical Markdown: the title line followed by the body.
    pub markdown: String,
    pub theme: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

/// A note version that arrived from another device (WebDAV).
#[derive(Debug, Clone, Copy)]
pub struct RemoteUpdate<'a> {
    pub id: &'a str,
    pub markdown: &'a str,
    pub theme: &'a str,
    /// The remote timestamp is kept as-is: stamping local time would make the
    /// note look newer than the file it came from, and the two devices would
    /// upload it back and forth forever.
    pub updated_at: i64,
    pub tags: &'a [String],
    pub pinned: bool,
}
