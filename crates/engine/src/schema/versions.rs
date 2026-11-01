//! Contracts and result types for the commands in `cmd/versions.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A non-blank folder path, with the handler's message for anything else.
fn folder<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) if !s.trim().is_empty() && s.len() < 4096 => Ok(s),
        _ => Err(serde::de::Error::custom("must be a folder path")),
    }
}

/// Text or null, with the handler's message for anything else.
fn optional_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) if s.len() <= 4096 => Ok(Some(s)),
        _ => Err(serde::de::Error::custom("must be text")),
    }
}

/// A count of 1 or more, with the handler's message for anything else.
fn limit<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        v => v.as_u64().filter(|n| *n > 0).map(Some).ok_or_else(|| serde::de::Error::custom("must be a whole number of versions, 1 or more")),
    }
}

/// A list of file paths, with the handler's message for anything else.
fn paths<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Array(a) if !a.is_empty() && a.len() <= 100_000 => a
            .into_iter()
            .map(|v| match v {
                Value::String(s) if s.len() <= 4096 => Ok(s),
                _ => Err(serde::de::Error::custom("must be a list of files in the folder")),
            })
            .collect(),
        _ => Err(serde::de::Error::custom("must be a list of files in the folder")),
    }
}

/// The folder whose versions to look at or start keeping.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VersionsFolderParams {
    /// The folder. Its designs and STEP files, in every sub-folder, are what its versions keep.
    #[serde(deserialize_with = "folder")]
    pub folder: String,
}

/// A part that differs from the latest version.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionChange {
    /// Relative to the folder, `/` between folders.
    pub path: String,
    /// `added` (new since the latest version), `changed` or `deleted`.
    pub change: String,
    /// The file is on this computer (false: only in iCloud Drive).
    pub local: bool,
}

/// What a check-in would save.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionsStatusReply {
    pub folder: String,
    /// The folder keeps versions (`versions.start`).
    pub tracked: bool,
    /// The latest version's id, once something is checked in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// Sorted by path.
    pub changes: Vec<VersionChange>,
    /// Designs and STEP files in the folder.
    pub files: u64,
    /// The folder held more files than versions keep (100,000); the rest aren't looked at.
    pub truncated: bool,
}

/// A part revision to record with a check-in.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VersionsRelease {
    /// The part, relative to the folder; one of the check-in's `paths`.
    pub path: String,
    /// Such as "Rev B".
    pub revision: String,
}

/// Save the named parts as a new version of the folder.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VersionsCheckInParams {
    #[serde(deserialize_with = "folder")]
    pub folder: String,
    /// What changed, shown in the history.
    pub message: String,
    /// The parts to check in, relative to the folder. A part no longer in the folder is
    /// removed from the new version.
    #[serde(deserialize_with = "paths")]
    pub paths: Vec<String>,
    /// Who checks in. Default: this computer's user name.
    #[serde(default, deserialize_with = "optional_text")]
    pub author: Option<String>,
    /// Also release one of the parts as a revision.
    #[serde(default)]
    pub release: Option<VersionsRelease>,
}

/// A file a version changed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionFile {
    pub path: String,
    /// `added`, `changed` or `deleted`.
    pub change: String,
}

/// The new version.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionsCheckInReply {
    pub commit: String,
    pub short: String,
    pub files: Vec<VersionFile>,
    /// The revision released with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// The versions of the folder, or of one part in it.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VersionsHistoryParams {
    #[serde(deserialize_with = "folder")]
    pub folder: String,
    /// Only the versions that changed this part (relative to the folder).
    #[serde(default, deserialize_with = "optional_text")]
    pub path: Option<String>,
    /// The most versions returned, newest first. Default: 200; at most 5000.
    #[serde(default, deserialize_with = "limit")]
    pub limit: Option<u64>,
}

/// A part revision a version is named.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionRevision {
    pub path: String,
    pub revision: String,
}

/// One version.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionEntry {
    pub id: String,
    pub short: String,
    pub message: String,
    pub author: String,
    /// Seconds since 1970 (UTC).
    pub time: i64,
    pub files: Vec<VersionFile>,
    /// Names of the whole folder at this version, such as "v1.3".
    pub versions: Vec<String>,
    /// Part revisions released at this version.
    pub revisions: Vec<VersionRevision>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionsHistoryReply {
    pub folder: String,
    /// Newest first.
    pub commits: Vec<VersionEntry>,
    /// There are older versions than the limit returned.
    pub more: bool,
}

/// Put one part, or every part, back as it was in a version.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VersionsRestoreParams {
    #[serde(deserialize_with = "folder")]
    pub folder: String,
    /// The version's id.
    pub commit: String,
    /// The part, relative to the folder. Default: every part that differs.
    #[serde(default, deserialize_with = "optional_text")]
    pub path: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionsRestoreReply {
    /// The files written back, relative to the folder.
    pub restored: Vec<String>,
}

/// Write a part as it was in a version to a new file.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VersionsCopyParams {
    #[serde(deserialize_with = "folder")]
    pub folder: String,
    pub commit: String,
    /// The part, relative to the folder.
    pub path: String,
    /// A new part in the folder (relative, must not exist): Save as New Part. Default: a copy
    /// outside the folder, to open and look at.
    #[serde(default, deserialize_with = "optional_text")]
    pub to: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionsCopyReply {
    /// The file written.
    pub file: String,
    /// The new part relative to the folder, when `to` named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Name a version: the whole folder (`version`), or a part's revision (`path` and `revision`).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VersionsNameParams {
    #[serde(deserialize_with = "folder")]
    pub folder: String,
    pub commit: String,
    /// Such as "v1.3" or "Sea trial 2".
    #[serde(default, deserialize_with = "optional_text")]
    pub version: Option<String>,
    /// The part whose revision this is, relative to the folder.
    #[serde(default, deserialize_with = "optional_text")]
    pub path: Option<String>,
    /// Such as "Rev B".
    #[serde(default, deserialize_with = "optional_text")]
    pub revision: Option<String>,
    #[serde(default, deserialize_with = "optional_text")]
    pub author: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VersionsNameReply {
    /// The tag's name in the history.
    pub tag: String,
    pub commit: String,
}

#[cfg(test)]
#[path = "versions_tests.rs"]
mod tests;
