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

/// Text or null, with the handler's message for anything else.
fn optional_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) if s.len() <= 4096 => Ok(Some(s)),
        _ => Err(serde::de::Error::custom("must be text")),
    }
}

/// `true`, `false` or null, with the handler's message for anything else.
fn optional_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(b)),
        _ => Err(serde::de::Error::custom("must be true or false")),
    }
}

/// Save a STEP, IGES or other CAD file as a SolidCraft library part beside it (the same name,
/// `.solidcraft`), with its properties and picture, without changing the open design.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PartsConvertParams {
    /// The CAD file.
    pub path: String,
    /// The part's name. Default: the file's name.
    #[serde(default, deserialize_with = "optional_text")]
    pub name: Option<String>,
    /// The part's part number.
    #[serde(default, deserialize_with = "optional_text")]
    pub part_number: Option<String>,
    /// The part's description.
    #[serde(default, deserialize_with = "optional_text")]
    pub description: Option<String>,
    /// Who makes the part.
    #[serde(default, deserialize_with = "optional_text")]
    pub organization: Option<String>,
    /// The part's revision.
    #[serde(default, deserialize_with = "optional_text")]
    pub revision: Option<String>,
    /// Turn a file whose header names a Y-up exporter (SolidWorks) a quarter turn about X, Y
    /// becoming Z. Default: false, the file is kept as it is.
    #[serde(default, deserialize_with = "optional_flag")]
    pub stand_up: Option<bool>,
    /// Make the design already beside the file again, in place under its own name (its links
    /// still find it): Reconvert. Default: false, which refuses.
    #[serde(default, deserialize_with = "optional_flag")]
    pub overwrite: Option<bool>,
}

/// The library part saved.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PartsConvertReply {
    /// The design, beside the CAD file.
    pub file: String,
    pub name: String,
    /// Solid bodies in the part.
    pub bodies: u64,
    /// The part was turned to stand up.
    pub stood_up: bool,
    /// `y` or `z`: the up axis the file's header names, when it names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exporter_up: Option<String>,
}

#[cfg(test)]
#[path = "parts_library_tests.rs"]
mod tests;
