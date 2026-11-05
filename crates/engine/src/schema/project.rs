//! Contracts and result types for the commands in `cmd/project.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A place in the project: text or null, with the handler's message for anything else.
fn place<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) if s.len() <= 4096 => Ok(Some(s)),
        _ => Err(serde::de::Error::custom("must be a place in the project, such as Electrical/pod.kicad_sch")),
    }
}

/// A file in the project: non-blank text.
fn file<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) if !s.trim().is_empty() && s.len() <= 4096 => Ok(s),
        _ => Err(serde::de::Error::custom("must be a file in the project, such as Electrical/pod.kicad_sch")),
    }
}

/// Search text: 1 to 200 characters.
fn query<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) if !s.trim().is_empty() && s.chars().count() <= 200 => Ok(s),
        _ => Err(serde::de::Error::custom("must be the text to look for, 1 to 200 characters")),
    }
}

/// A whole number in `lo..=hi`, or null.
fn within<'de, D: serde::Deserializer<'de>>(d: D, lo: u64, hi: u64, what: &str) -> Result<Option<u64>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        v => v.as_u64().filter(|n| (lo..=hi).contains(n)).map(Some).ok_or_else(|| serde::de::Error::custom(format!("must be {what}"))),
    }
}

fn list_limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    within(d, 1, 5000, "1 to 5000 files")
}

fn search_limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    within(d, 1, 1000, "1 to 1000 matches")
}

fn chars<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    within(d, 1000, 100_000, "1000 to 100000 characters")
}

fn offset<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    within(d, 0, u64::from(u32::MAX), "a character count from 0 (a reply's next_offset)")
}

/// List the files in the project folder.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ProjectFilesParams {
    /// A folder in the project to list (default: all of it).
    #[serde(default, deserialize_with = "place")]
    pub path: Option<String>,
    /// Only files whose name holds this text (any case), or of this kind: design, cad, mesh, pdf,
    /// kicad, text, word, office, picture.
    #[serde(default, deserialize_with = "place")]
    pub filter: Option<String>,
    /// The most files listed. Default: 500; 1 to 5000.
    #[serde(default, deserialize_with = "list_limit")]
    pub limit: Option<u64>,
}

/// A file in the project.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectFileEntry {
    /// Its place in the project, `/` between folders: what project.read takes.
    pub path: String,
    /// design, cad, mesh, pdf, kicad, text, word, office, picture or other.
    pub kind: String,
    pub bytes: u64,
    /// When it last changed, in seconds since 1970.
    pub modified: i64,
}

/// What `project.files` found.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectFilesReply {
    /// The project's name (its folder's).
    pub project: String,
    /// The project folder, a full path.
    pub folder: String,
    /// The project's notes for the assistant (PROJECT.md, else README.md), when it has them:
    /// read them first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    pub files: Vec<ProjectFileEntry>,
    /// Files that matched, listed or not.
    pub total: u64,
    /// Not every matching file is listed: narrow with path or filter.
    pub truncated: bool,
}

/// Look for text in the project's file names and text files.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ProjectSearchParams {
    /// The text to look for, any case: a part number, a reference such as J3, a net or a word.
    #[serde(deserialize_with = "query")]
    pub query: String,
    /// A folder in the project to look in (default: all of it).
    #[serde(default, deserialize_with = "place")]
    pub path: Option<String>,
    /// The most matches. Default: 100; 1 to 1000.
    #[serde(default, deserialize_with = "search_limit")]
    pub limit: Option<u64>,
}

/// A place `project.search` found.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectHit {
    pub path: String,
    /// The line, from 1; none when the file's name matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    /// The line, cut short; or the file's kind when its name matched.
    pub text: String,
}

/// What `project.search` found.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectSearchReply {
    pub matches: Vec<ProjectHit>,
    /// Text files looked through (PDFs are not searched: read them with project.read).
    pub files_searched: u64,
    pub truncated: bool,
}

/// Read a file in the project.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ProjectReadParams {
    /// The file, as project.files lists it.
    #[serde(deserialize_with = "file")]
    pub path: String,
    /// Where to start, in characters (a reply's next_offset). Default: 0.
    #[serde(default, deserialize_with = "offset")]
    pub offset: Option<u64>,
    /// The most characters returned. Default: 20,000; 1,000 to 100,000.
    #[serde(default, deserialize_with = "chars")]
    pub max_chars: Option<u64>,
    /// A KiCad schematic or board as its own text, not summarised. Default: false.
    #[serde(default)]
    pub raw: Option<bool>,
}

/// What `project.read` read.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectReadReply {
    pub path: String,
    pub kind: String,
    pub bytes: u64,
    pub modified: i64,
    /// A KiCad schematic's or board's summary: its parts (reference, value, symbol or footprint,
    /// fields such as MPN, place and side), sheets, labels or nets, layers and outline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Value>,
    /// The file's text, from offset: a document's words, a PDF's text page by page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A PDF's pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<u64>,
    pub offset: u64,
    /// Where the next part starts, when there is more: call again with it as offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    /// What can't be read here, and what to do instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The text of a PDF or Word file.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FileTextParams {
    /// The file, a full path: a .pdf or .docx.
    #[serde(deserialize_with = "file")]
    pub path: String,
    /// Where to start, in characters. Default: 0.
    #[serde(default, deserialize_with = "offset")]
    pub offset: Option<u64>,
    /// The most characters returned. Default: 20,000; 1,000 to 100,000.
    #[serde(default, deserialize_with = "chars")]
    pub max_chars: Option<u64>,
}

/// What `file.text` read.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FileTextReply {
    pub path: String,
    pub text: String,
    /// A PDF's pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<u64>,
    pub offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
}
