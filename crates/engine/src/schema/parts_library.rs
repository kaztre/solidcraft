//! Contracts and result types for the commands in `cmd/parts_library.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A non-blank text, with the handler's message for anything else.
fn folder<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) if !s.trim().is_empty() && s.len() < 4096 => Ok(s),
        _ => Err(serde::de::Error::custom("must be a folder path")),
    }
}

/// Text or null, with the handler's message for anything else.
fn query<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        _ => Err(serde::de::Error::custom("must be text")),
    }
}

/// A count of 1 or more, with the handler's message for anything else.
fn limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        v => v.as_u64().filter(|n| *n > 0).map(Some).ok_or_else(|| serde::de::Error::custom("must be a whole number of parts, 1 or more")),
    }
}

/// List and search the parts in a parts library folder, without changing the design.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PartsSearchParams {
    /// The library folder. Its sub-folders are the library's categories.
    #[serde(deserialize_with = "folder")]
    pub folder: String,
    /// Words that must all appear in a part's name, folders, file type, part number or
    /// description, any case (`_` in names counts as a space). Default: every part.
    #[serde(default, deserialize_with = "query")]
    pub query: Option<String>,
    /// The most parts returned, best matches first. Default: 200; at most 5000.
    #[serde(default, deserialize_with = "limit")]
    pub limit: Option<u64>,
}

/// One part of the library.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LibraryPart {
    /// Relative to the library folder, `/` between folders.
    pub path: String,
    /// The file on this computer: `component.insert_design` takes a design's, `file.insert_step`
    /// a CAD file's.
    pub file: String,
    /// The file name without its extension.
    pub name: String,
    /// The folders it is in, relative to the library folder ("" at the top).
    pub folder: String,
    /// `design`: a SolidCraft design, inserted as a linked component. `cad`: STEP, IGES or another
    /// CAD file, whose bodies are copied in.
    pub kind: String,
    /// The file's extension, lower-case.
    pub format: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// The parts that matched.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PartsSearchReply {
    pub folder: String,
    /// Best matches first, then by folder and name.
    pub parts: Vec<LibraryPart>,
    /// How many parts matched (more than `parts` when the limit cut the list).
    pub matched: u64,
    /// How many parts the library has.
    pub total: u64,
    /// The folder held more parts than one library lists (50,000), so some are missing.
    pub truncated: bool,
}

#[cfg(test)]
#[path = "parts_library_tests.rs"]
mod tests;
