//! Contracts and result types for the commands in `cmd/web.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A web address: non-blank text, with the handler's message for anything else.
fn address<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) if !s.trim().is_empty() && s.len() <= 4096 => Ok(s),
        _ => Err(serde::de::Error::custom("must be a web address (https://…)")),
    }
}

/// Text or null, with the handler's message for anything else.
fn text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) if s.len() <= 4096 => Ok(Some(s)),
        _ => Err(serde::de::Error::custom("must be text")),
    }
}

/// A count of characters, 1,000 to 100,000, with the handler's message for anything else.
fn chars<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        v => v
            .as_u64()
            .filter(|n| (1_000..=100_000).contains(n))
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("must be 1000 to 100000 characters")),
    }
}

/// Read a web page: its title, its text and its links.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct WebFetchParams {
    /// The page's address: https:// only, never one on this computer's network.
    #[serde(deserialize_with = "address")]
    pub url: String,
    /// The most characters of text returned. Default: 20,000; 1,000 to 100,000.
    #[serde(default, deserialize_with = "chars")]
    pub max_chars: Option<u64>,
}

/// A link on a page.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WebLink {
    /// The link's text, as the page shows it.
    pub text: String,
    /// Where it goes, as a full address.
    pub url: String,
}

/// What `web.fetch` read.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WebFetchReply {
    /// The address the page came from, after any redirects.
    pub url: String,
    /// The HTTP status.
    pub status: u16,
    /// The page's title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The page's text, without its markup.
    pub text: String,
    /// Whether the text was cut at `max_chars`.
    pub truncated: bool,
    /// The page's links, in order, each once (at most 300).
    pub links: Vec<WebLink>,
    /// The links that name a CAD file or a zip: what `file.download` takes.
    pub cad_links: Vec<String>,
}

/// Download the CAD files a web address gives into a folder.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FileDownloadParams {
    /// The file's address: https:// only. A STEP, IGES or mesh file, or a zip holding them.
    #[serde(deserialize_with = "address")]
    pub url: String,
    /// The folder to save in (a full path). Default: the Downloads folder of the first Parts
    /// Library, else Downloads/SolidCraft in the home folder.
    #[serde(default, deserialize_with = "text")]
    pub folder: Option<String>,
    /// The name to save a single file under. Default: the name the site gives.
    #[serde(default, deserialize_with = "text")]
    pub name: Option<String>,
}

/// A file `file.download` saved.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DownloadedFile {
    /// Where it was saved.
    pub file: String,
    /// Its type, by its extension: `step`, `stp`, `igs`, `stl`, `3mf`, …
    pub format: String,
    pub bytes: u64,
}

/// What `file.download` saved.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FileDownloadReply {
    /// The address the files came from, after any redirects.
    pub from: String,
    /// The folder they were saved in.
    pub folder: String,
    /// The CAD files saved: one, or those a zip held.
    pub files: Vec<DownloadedFile>,
}
