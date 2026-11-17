//! Data model for plum Note rows.

#[derive(Debug, Clone)]
pub struct NoteSummary {
    pub id: String,
    /// First non-empty line, markup stripped (for the list view).
    pub title: String,
    /// Brief snippet of subsequent text for preview.
    pub snippet: String,
    pub theme: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NoteRow {
    pub id: String,
    pub text: String,
    pub theme: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub deleted_at: Option<i64>,
}
