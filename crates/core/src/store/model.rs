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
    /// Last change to the text; the note list is ordered by it.
    pub updated_at: i64,
    /// Last change to the colour, tags or pin made on its own, 0 if none.
    pub meta_updated_at: i64,
}

impl Note {
    /// The note's version for sync: its newest change of any kind.
    pub fn version(&self) -> i64 {
        self.updated_at.max(self.meta_updated_at)
    }
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
    /// Kept as-is for the same reason.
    pub meta_updated_at: i64,
    pub tags: &'a [String],
    pub pinned: bool,
}

impl RemoteUpdate<'_> {
    /// The version's newest change of any kind, as in [`Note::version`].
    pub fn version(&self) -> i64 {
        self.updated_at.max(self.meta_updated_at)
    }
}
