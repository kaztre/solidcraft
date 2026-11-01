//! Versions of the parts in a folder (docs/versions.md). Each check-in saves the parts it names
//! as a new version of the folder, in git's own format (zlib objects named by their SHA-1, trees,
//! commits, annotated tags), written here so that nothing has to be installed and the history
//! can later be shared through any git host. Only designs and STEP files are kept; meshes,
//! drawings and everything else in the folder are left alone.
//!
//! The history lives in the folder's own `.git` when it has one, and otherwise in a folder the
//! caller names (SolidCraft's data folder): a synced folder such as iCloud Drive or Google Drive
//! would upload, and can corrupt, a history kept inside it. Linear history only: one line of
//! versions, on `main`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

/// The file types a folder's versions keep, lower-case.
pub const KEPT_EXTS: [&str; 4] = [crate::DESIGN_EXT, crate::LEGACY_DESIGN_EXT, "step", "stp"];
/// The most files one folder's versions keep.
pub const MAX_FILES: usize = 100_000;
/// The largest file a check-in keeps (bigger STEP files are refused, not cut short).
pub const MAX_FILE_BYTES: u64 = 2 << 30;
/// Folders and files looked at before a walk stops, and how deep it goes.
const MAX_VISITED: usize = 1_000_000;
const MAX_DEPTH: usize = 32;
/// Commits a history walk reads at most, and the deepest tree it follows.
const MAX_WALK: usize = 1_000_000;
const MAX_TREE_DEPTH: usize = 64;
/// Commits, trees and tags are small; anything bigger is not ours.
const MAX_META_BYTES: u64 = 64 << 20;

const BRANCH: &str = "refs/heads/main";
const FILE_CACHE: &str = "solidcraft-files.json";
const NOTES: &str = "solidcraft-notes.json";

#[derive(Debug, thiserror::Error)]
pub enum VersionsError {
    /// The folder or the history couldn't be read or written.
    #[error("{0}")]
    Io(String),
    /// The history holds something this reader doesn't understand.
    #[error("the version history is damaged: {0}")]
    Damaged(String),
    /// The request can't be done (no such version, nothing to check in, a bad name).
    #[error("{0}")]
    Refused(String),
}

pub type Result<T> = std::result::Result<T, VersionsError>;

fn io(what: impl std::fmt::Display, e: std::io::Error) -> VersionsError {
    VersionsError::Io(format!("{what}: {e}"))
}

fn damaged(what: impl Into<String>) -> VersionsError {
    VersionsError::Damaged(what.into())
}

fn refused(what: impl Into<String>) -> VersionsError {
    VersionsError::Refused(what.into())
}

/// Who checks in, as the history shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    pub email: String,
}

impl Author {
    /// Names safe for a commit header: no angle brackets or line breaks, never empty.
    pub fn new(name: &str, email: &str) -> Author {
        let clean = |s: &str| s.chars().filter(|c| !matches!(c, '<' | '>' | '\n' | '\r' | '\0')).collect::<String>().trim().to_string();
        let name = clean(name);
        let name = if name.is_empty() { "SolidCraft".to_string() } else { name.chars().take(200).collect() };
        let email = clean(email);
        let email = if email.is_empty() { "solidcraft@localhost".to_string() } else { email.chars().take(200).collect() };
        Author { name, email }
    }
}

/// How a file differs from the version before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Added,
    Changed,
    Deleted,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Changed => "changed",
            Change::Deleted => "deleted",
        }
    }
}

/// A kept file that differs from the latest version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Relative to the folder, `/` between folders.
    pub path: String,
    pub change: Change,
    /// The file's contents are on this computer (not only in iCloud Drive).
    pub local: bool,
}

/// What a check-in would save.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// The latest version, if there is one.
    pub head: Option<String>,
    /// Sorted by path.
    pub changes: Vec<FileChange>,
    /// Kept files in the folder.
    pub files: usize,
    /// The folder held more than [`MAX_FILES`] kept files; files past those aren't looked at.
    pub truncated: bool,
}

/// One version of the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub author: Author,
    /// Seconds since 1970 (UTC).
    pub time: i64,
    pub message: String,
}

/// A version in the history, with the files it changed.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub commit: Commit,
    /// Sorted by path.
    pub files: Vec<(String, Change)>,
}

/// A name given to a version: a version of the whole folder (`version`), or a part's revision
/// (`part` and `revision`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The tag's name in the history.
    pub name: String,
    /// The version it names.
    pub commit: String,
    pub version: Option<String>,
    pub part: Option<String>,
    pub revision: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct TagNote {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

/// A kept file in the folder.
#[derive(Clone, Debug)]
struct WorkFile {
    path: String,
    size: u64,
    modified: u128,
    local: bool,
}

/// What the last look at a file found, so an unchanged file isn't read again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
    size: u64,
    modified: u128,
    id: String,
}

/// One folder's versions.
#[derive(Clone, Debug)]
pub struct Store {
    /// The folder whose parts are kept.
    pub work: PathBuf,
    /// The history (a git directory).
    pub git: PathBuf,
}

/// Is this file one a folder's versions keep?
pub fn is_kept(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    KEPT_EXTS.contains(&ext.as_str())
}

/// A relative `/` path inside the folder: no empty, `.`, `..` or hidden parts, no `\`.
pub fn check_path(path: &str) -> Result<()> {
    let bad = path.is_empty()
        || path.len() > 4096
        || path.contains(['\\', '\0', '\n', '\r'])
        || path.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.starts_with('.'));
    if bad { Err(refused(format!("`{path}` isn't a file inside the folder"))) } else { Ok(()) }
}

/// A short form of a version's id, as the history shows it.
pub fn short(id: &str) -> &str {
    id.get(..7).unwrap_or(id)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn is_id(id: &str) -> bool {
    id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// A name git accepts for a tag: letters, digits, `.`, `_` and `-`, starting with a letter or a
/// digit.
fn ref_name(text: &str) -> String {
    let mut s: String = text.trim().chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
    while s.contains("..") {
        s = s.replace("..", ".");
    }
    let s = s.trim_matches(|c: char| !c.is_ascii_alphanumeric()).chars().take(80).collect::<String>();
    let s = s.trim_end_matches(".lock").to_string();
    if s.is_empty() { "version".to_string() } else { s }
}

fn modified_ns(meta: &std::fs::Metadata) -> u128 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos())
}

/// Whether a file's contents are on this computer: macOS marks a file iCloud Drive keeps only
/// in the cloud as dataless, and reading it downloads it.
fn is_local(meta: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const SF_DATALESS: u32 = 0x4000_0000;
        meta.st_flags() & SF_DATALESS == 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = meta;
        true
    }
}

/// Writes `bytes` beside `path` and moves it into place, so a reader never sees half a file.
fn write_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| refused(format!("{} has no folder", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| io(dir.display(), e))?;
    let tmp = temp_beside(path);
    std::fs::write(&tmp, bytes).map_err(|e| io(tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(path.display(), e)
    })
}

fn temp_beside(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.{}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// A tree being built: files by name, and folders.
#[derive(Default)]
struct Dir {
    files: BTreeMap<String, String>,
    dirs: BTreeMap<String, Dir>,
}

fn dir_id(e: Option<&TreeEntry>) -> Option<&str> {
    e.filter(|e| e.is_dir()).map(|e| e.id.as_str())
}

fn file_id(e: Option<&TreeEntry>) -> Option<&str> {
    e.filter(|e| !e.is_dir()).map(|e| e.id.as_str())
}

/// One entry of a stored tree.
struct TreeEntry {
    mode: String,
    name: String,
    id: String,
}

impl TreeEntry {
    fn is_dir(&self) -> bool {
        self.mode == "40000" || self.mode == "040000"
    }
}

impl Store {
    /// The folder's history: its own `.git` when it has one, otherwise `history` (made by
    /// [`Store::start`]).
    pub fn locate(work: &Path, history: &Path) -> Store {
        let own = work.join(".git");
        let git = if own.join("HEAD").is_file() { own } else { history.to_path_buf() };
        Store { work: work.to_path_buf(), git }
    }

    /// Does the folder have versions yet?
    pub fn exists(&self) -> bool {
        self.git.join("HEAD").is_file() && self.git.join("objects").is_dir()
    }

    /// Starts keeping versions of the folder: an empty history. Nothing is checked in yet.
    pub fn start(&self) -> Result<()> {
        if !self.work.is_dir() {
            return Err(refused(format!("{} isn't a folder", self.work.display())));
        }
        if self.exists() {
            return Ok(());
        }
        for dir in ["objects", "refs/heads", "refs/tags", "info"] {
            let d = self.git.join(dir);
            std::fs::create_dir_all(&d).map_err(|e| io(d.display(), e))?;
        }
        let work = self.work.to_string_lossy().replace('\\', "/").replace('"', "\\\"");
        let config = format!(
            "[core]\n\trepositoryformatversion = 0\n\tfilemode = false\n\tbare = false\n\tlogallrefupdates = true\n\tworktree = \"{work}\"\n"
        );
        write_replace(&self.git.join("config"), config.as_bytes())?;
        write_replace(&self.git.join("description"), b"SolidCraft versions\n")?;
        // Everything but designs and STEP files stays out, for git tools that open the folder.
        let exclude = "*\n!*/\n!*.solidcraft\n!*.solvecraft\n!*.step\n!*.STEP\n!*.stp\n!*.STP\n.*\n";
        write_replace(&self.git.join("info/exclude"), exclude.as_bytes())?;
        write_replace(&self.git.join("HEAD"), format!("ref: {BRANCH}\n").as_bytes())
    }

    // ---- objects ----

    fn object_path(&self, id: &str) -> Result<PathBuf> {
        if !is_id(id) {
            return Err(refused(format!("`{id}` isn't a version id")));
        }
        Ok(self.git.join("objects").join(&id[..2]).join(&id[2..]))
    }

    fn write_object(&self, kind: &str, data: &[u8]) -> Result<String> {
        let header = format!("{kind} {}\0", data.len());
        let mut h = Sha1::new();
        h.update(header.as_bytes());
        h.update(data);
        let id = hex(&h.finalize());
        let path = self.object_path(&id)?;
        if path.is_file() {
            return Ok(id);
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(header.as_bytes()).and_then(|_| z.write_all(data)).map_err(|e| io("compressing", e))?;
        let bytes = z.finish().map_err(|e| io("compressing", e))?;
        write_replace(&path, &bytes)?;
        Ok(id)
    }

    /// Stores a file as a blob without holding it all in memory, and returns its id.
    fn write_blob_from(&self, file: &Path) -> Result<String> {
        let meta = std::fs::metadata(file).map_err(|e| io(file.display(), e))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(refused(format!("{} is over {} GB, the largest file versions keep", file.display(), MAX_FILE_BYTES >> 30)));
        }
        let objects = self.git.join("objects");
        std::fs::create_dir_all(&objects).map_err(|e| io(objects.display(), e))?;
        let tmp = temp_beside(&objects.join("blob"));
        let result = (|| {
            let mut input = std::fs::File::open(file).map_err(|e| io(file.display(), e))?;
            let out = std::fs::File::create(&tmp).map_err(|e| io(tmp.display(), e))?;
            let mut z = flate2::write::ZlibEncoder::new(std::io::BufWriter::new(out), flate2::Compression::fast());
            let header = format!("blob {}\0", meta.len());
            let mut h = Sha1::new();
            h.update(header.as_bytes());
            z.write_all(header.as_bytes()).map_err(|e| io(tmp.display(), e))?;
            let mut buf = vec![0u8; 1 << 20];
            let mut total = 0u64;
            loop {
                let n = input.read(&mut buf).map_err(|e| io(file.display(), e))?;
                let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
                total += n as u64;
                if total > meta.len() {
                    return Err(refused(format!("{} changed while it was read; check in again", file.display())));
                }
                h.update(chunk);
                z.write_all(chunk).map_err(|e| io(tmp.display(), e))?;
            }
            if total != meta.len() {
                return Err(refused(format!("{} changed while it was read; check in again", file.display())));
            }
            let mut w = z.finish().map_err(|e| io(tmp.display(), e))?;
            w.flush().map_err(|e| io(tmp.display(), e))?;
            Ok(hex(&h.finalize()))
        })();
        let id = match result {
            Ok(id) => id,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        let path = self.object_path(&id)?;
        if path.is_file() {
            let _ = std::fs::remove_file(&tmp);
            return Ok(id);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io(dir.display(), e))?;
        }
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io(path.display(), e)
        })?;
        Ok(id)
    }

    /// The id a file would have as a blob, read in pieces.
    fn hash_file(file: &Path) -> Result<String> {
        let meta = std::fs::metadata(file).map_err(|e| io(file.display(), e))?;
        let mut input = std::fs::File::open(file).map_err(|e| io(file.display(), e))?;
        let mut h = Sha1::new();
        h.update(format!("blob {}\0", meta.len()).as_bytes());
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            let n = input.read(&mut buf).map_err(|e| io(file.display(), e))?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
            total += n as u64;
            h.update(chunk);
        }
        if total != meta.len() {
            return Err(refused(format!("{} changed while it was read", file.display())));
        }
        Ok(hex(&h.finalize()))
    }

    /// Opens an object: its kind, its length, and a reader positioned at its contents.
    fn open_object(&self, id: &str) -> Result<(String, u64, flate2::read::ZlibDecoder<std::io::BufReader<std::fs::File>>)> {
        let path = self.object_path(id)?;
        let file = std::fs::File::open(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => damaged(format!("version data {} is missing", short(id))),
            _ => io(path.display(), e),
        })?;
        let mut z = flate2::read::ZlibDecoder::new(std::io::BufReader::new(file));
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = z.read(&mut byte).map_err(|e| io(path.display(), e))?;
            if n == 0 || header.len() > 64 {
                return Err(damaged(format!("{} has no header", short(id))));
            }
            if byte[0] == 0 {
                break;
            }
            header.push(byte[0]);
        }
        let header = String::from_utf8(header).map_err(|_| damaged(format!("{} has a bad header", short(id))))?;
        let (kind, len) = header.split_once(' ').ok_or_else(|| damaged(format!("{} has a bad header", short(id))))?;
        let len = len.parse::<u64>().map_err(|_| damaged(format!("{} has a bad length", short(id))))?;
        Ok((kind.to_string(), len, z))
    }

    /// Reads a small object (a commit, tree or tag) whole.
    fn read_small(&self, id: &str, want: &str) -> Result<Vec<u8>> {
        let (kind, len, reader) = self.open_object(id)?;
        if kind != want {
            return Err(damaged(format!("{} is a {kind}, not a {want}", short(id))));
        }
        if len > MAX_META_BYTES {
            return Err(damaged(format!("{} is too big for a {want}", short(id))));
        }
        let mut data = Vec::new();
        reader.take(len + 1).read_to_end(&mut data).map_err(|e| io(short(id), e))?;
        if data.len() as u64 != len {
            return Err(damaged(format!("{} is cut short", short(id))));
        }
        Ok(data)
    }

    fn kind_of(&self, id: &str) -> Result<String> {
        Ok(self.open_object(id)?.0)
    }

    /// Writes a stored file to `dest`, replacing it only once all of it is written.
    fn copy_blob(&self, id: &str, dest: &Path) -> Result<()> {
        let (kind, len, reader) = self.open_object(id)?;
        if kind != "blob" {
            return Err(damaged(format!("{} isn't a file", short(id))));
        }
        let dir = dest.parent().ok_or_else(|| refused(format!("{} has no folder", dest.display())))?;
        std::fs::create_dir_all(dir).map_err(|e| io(dir.display(), e))?;
        let tmp = temp_beside(dest);
        let result = (|| {
            let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(|e| io(tmp.display(), e))?);
            let n = std::io::copy(&mut reader.take(len + 1), &mut out).map_err(|e| io(dest.display(), e))?;
            if n != len {
                return Err(damaged(format!("{} is cut short", short(id))));
            }
            out.flush().map_err(|e| io(tmp.display(), e))?;
            std::fs::rename(&tmp, dest).map_err(|e| io(dest.display(), e))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    // ---- trees ----

    fn read_tree(&self, id: &str) -> Result<Vec<TreeEntry>> {
        let data = self.read_small(id, "tree")?;
        let mut entries = Vec::new();
        let mut rest = data.as_slice();
        while !rest.is_empty() {
            let space = rest.iter().position(|b| *b == b' ').ok_or_else(|| damaged(format!("tree {} is bad", short(id))))?;
            let (mode, after) = rest.split_at(space);
            let after = after.get(1..).unwrap_or_default();
            let nul = after.iter().position(|b| *b == 0).ok_or_else(|| damaged(format!("tree {} is bad", short(id))))?;
            let (name, after) = after.split_at(nul);
            let raw = after.get(1..21).ok_or_else(|| damaged(format!("tree {} is cut short", short(id))))?;
            rest = after.get(21..).unwrap_or_default();
            let name = String::from_utf8_lossy(name).into_owned();
            entries.push(TreeEntry { mode: String::from_utf8_lossy(mode).into_owned(), name, id: hex(raw) });
        }
        Ok(entries)
    }

    /// Every file in a tree, by path.
    fn flatten(&self, tree: &str) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        self.flatten_into(tree, "", &mut out, 0)?;
        Ok(out)
    }

    fn flatten_into(&self, tree: &str, prefix: &str, out: &mut BTreeMap<String, String>, depth: usize) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            return Err(damaged("folders nest too deep"));
        }
        for e in self.read_tree(tree)? {
            let path = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}/{}", e.name) };
            if e.is_dir() {
                self.flatten_into(&e.id, &path, out, depth + 1)?;
            } else if out.len() < MAX_FILES {
                out.insert(path, e.id);
            }
        }
        Ok(())
    }

    /// The files two trees disagree on, skipping folders that are the same in both.
    fn diff_trees(&self, old: Option<&str>, new: Option<&str>, prefix: &str, out: &mut Vec<(String, Change)>, depth: usize) -> Result<()> {
        if old == new || depth > MAX_TREE_DEPTH {
            return Ok(());
        }
        let read = |id: Option<&str>| -> Result<BTreeMap<String, TreeEntry>> {
            Ok(match id {
                Some(id) => self.read_tree(id)?.into_iter().map(|e| (e.name.clone(), e)).collect(),
                None => BTreeMap::new(),
            })
        };
        let (a, b) = (read(old)?, read(new)?);
        let names: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for name in names {
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let (x, y) = (a.get(name), b.get(name));
            self.diff_trees(dir_id(x), dir_id(y), &path, out, depth + 1)?;
            match (file_id(x), file_id(y)) {
                (None, Some(_)) => out.push((path, Change::Added)),
                (Some(_), None) => out.push((path, Change::Deleted)),
                (Some(p), Some(q)) if p != q => out.push((path, Change::Changed)),
                _ => {}
            }
        }
        Ok(())
    }

    /// The stored file at `path` in `tree`, if there is one.
    fn find(&self, tree: &str, path: &str) -> Result<Option<String>> {
        let mut tree = tree.to_string();
        let parts: Vec<&str> = path.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let Some(e) = self.read_tree(&tree)?.into_iter().find(|e| e.name == *part) else { return Ok(None) };
            match (last, e.is_dir()) {
                (true, false) => return Ok(Some(e.id)),
                (false, true) => tree = e.id,
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    fn write_tree(&self, files: &BTreeMap<String, String>) -> Result<String> {
        let mut root = Dir::default();
        for (path, id) in files {
            let mut dir = &mut root;
            let mut parts = path.split('/').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    dir.files.insert(part.to_string(), id.clone());
                } else {
                    dir = dir.dirs.entry(part.to_string()).or_default();
                }
            }
        }
        self.write_dir(&root)
    }

    fn write_dir(&self, dir: &Dir) -> Result<String> {
        // Git orders a folder's entries by name, a sub-folder's name as if it ended in `/`.
        let mut entries: Vec<(Vec<u8>, &str, &str, String)> = Vec::new();
        for (name, id) in &dir.files {
            entries.push((name.as_bytes().to_vec(), "100644", name, id.clone()));
        }
        for (name, sub) in &dir.dirs {
            let id = self.write_dir(sub)?;
            let mut key = name.as_bytes().to_vec();
            key.push(b'/');
            entries.push((key, "40000", name, id));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut data = Vec::new();
        for (_, mode, name, id) in entries {
            data.extend_from_slice(mode.as_bytes());
            data.push(b' ');
            data.extend_from_slice(name.as_bytes());
            data.push(0);
            for i in 0..20 {
                let byte = id.get(i * 2..i * 2 + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(|| damaged("a bad id"))?;
                data.push(byte);
            }
        }
        self.write_object("tree", &data)
    }

    // ---- commits and refs ----

    /// The latest version, if anything has been checked in.
    pub fn head(&self) -> Result<Option<String>> {
        let head = match std::fs::read_to_string(self.git.join("HEAD")) {
            Ok(h) => h,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io("HEAD", e)),
        };
        let head = head.trim();
        let Some(name) = head.strip_prefix("ref: ") else {
            return Ok(is_id(head).then(|| head.to_string()));
        };
        self.read_ref(name.trim())
    }

    fn head_ref(&self) -> String {
        std::fs::read_to_string(self.git.join("HEAD"))
            .ok()
            .and_then(|h| h.trim().strip_prefix("ref: ").map(|r| r.trim().to_string()))
            .filter(|r| r.starts_with("refs/heads/") && !r.contains(".."))
            .unwrap_or_else(|| BRANCH.to_string())
    }

    fn read_ref(&self, name: &str) -> Result<Option<String>> {
        if name.contains("..") {
            return Ok(None);
        }
        match std::fs::read_to_string(self.git.join(name)) {
            Ok(id) => {
                let id = id.trim();
                return Ok(is_id(id).then(|| id.to_string()));
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(name, e)),
            Err(_) => {}
        }
        // A folder cloned with git may keep its refs packed.
        let packed = std::fs::read_to_string(self.git.join("packed-refs")).unwrap_or_default();
        Ok(packed.lines().find_map(|line| line.split_once(' ').filter(|(_, r)| *r == name).map(|(id, _)| id.to_string())).filter(|id| is_id(id)))
    }

    pub fn read_commit(&self, id: &str) -> Result<Commit> {
        let data = self.read_small(id, "commit")?;
        let text = String::from_utf8_lossy(&data);
        let (headers, message) = text.split_once("\n\n").unwrap_or((&text, ""));
        let mut c = Commit {
            id: id.to_string(),
            tree: String::new(),
            parents: Vec::new(),
            author: Author::new("", ""),
            time: 0,
            message: message.trim_end().to_string(),
        };
        for line in headers.lines() {
            if let Some(t) = line.strip_prefix("tree ") {
                c.tree = t.trim().to_string();
            } else if let Some(p) = line.strip_prefix("parent ") {
                c.parents.push(p.trim().to_string());
            } else if let Some(a) = line.strip_prefix("author ") {
                let (who, when) = a.rsplit_once('>').unwrap_or((a, ""));
                let (name, email) = who.split_once('<').unwrap_or((who, ""));
                c.author = Author::new(name, email);
                c.time = when.split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(0);
            }
        }
        if !is_id(&c.tree) {
            return Err(damaged(format!("version {} names no files", short(id))));
        }
        Ok(c)
    }

    fn head_files(&self) -> Result<(Option<Commit>, BTreeMap<String, String>)> {
        match self.head()? {
            Some(id) => {
                let c = self.read_commit(&id)?;
                let files = self.flatten(&c.tree)?;
                Ok((Some(c), files))
            }
            None => Ok((None, BTreeMap::new())),
        }
    }

    fn write_commit(&self, tree: &str, parent: Option<&str>, author: &Author, time: i64, message: &str) -> Result<String> {
        let mut text = format!("tree {tree}\n");
        if let Some(p) = parent {
            text.push_str(&format!("parent {p}\n"));
        }
        let who = format!("{} <{}> {time} +0000", author.name, author.email);
        text.push_str(&format!("author {who}\ncommitter {who}\n\n{}\n", message.trim_end()));
        self.write_object("commit", text.as_bytes())
    }

    // ---- the folder ----

    fn walk(&self) -> Result<(Vec<WorkFile>, bool)> {
        let mut files = Vec::new();
        let mut visited = 0usize;
        let mut truncated = false;
        let mut stack = vec![(self.work.clone(), String::new(), 0usize)];
        while let Some((dir, prefix, depth)) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if prefix.is_empty() => return Err(io(dir.display(), e)),
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited > MAX_VISITED || files.len() >= MAX_FILES {
                    truncated = true;
                    break;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name.contains(['\\', '\n', '\r']) {
                    continue;
                }
                let Ok(kind) = entry.file_type() else { continue };
                let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                if kind.is_dir() {
                    if depth < MAX_DEPTH {
                        stack.push((entry.path(), path, depth + 1));
                    }
                } else if kind.is_file() && is_kept(&name) {
                    let Ok(meta) = entry.metadata() else { continue };
                    files.push(WorkFile { path, size: meta.len(), modified: modified_ns(&meta), local: is_local(&meta) });
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok((files, truncated))
    }

    fn load_seen(&self) -> BTreeMap<String, Seen> {
        std::fs::read(self.git.join(FILE_CACHE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save_seen(&self, seen: &BTreeMap<String, Seen>) {
        if let Ok(bytes) = serde_json::to_vec(seen) {
            // Only a cache: losing it means files are read again.
            let _ = write_replace(&self.git.join(FILE_CACHE), &bytes);
        }
    }

    /// What differs from the latest version. Unchanged files aren't read again; a file only in
    /// iCloud Drive is never downloaded to find out (it counts as changed when its size or date
    /// did).
    pub fn status(&self) -> Result<Status> {
        if !self.exists() {
            return Err(refused(format!("{} has no versions yet", self.work.display())));
        }
        let (head, kept) = self.head_files()?;
        let (work, truncated) = self.walk()?;
        let mut seen = self.load_seen();
        let before = seen.clone();
        let mut changes = Vec::new();
        let mut present = BTreeSet::new();
        for f in &work {
            present.insert(f.path.as_str());
            let known = seen.get(&f.path).filter(|s| s.size == f.size && s.modified == f.modified).map(|s| s.id.clone());
            let id = match known {
                Some(id) => Some(id),
                None if f.local => match Store::hash_file(&self.work.join(&f.path)) {
                    Ok(id) => {
                        seen.insert(f.path.clone(), Seen { size: f.size, modified: f.modified, id: id.clone() });
                        Some(id)
                    }
                    Err(_) => None,
                },
                None => None,
            };
            let change = match (kept.get(&f.path), id) {
                (None, _) => Some(Change::Added),
                (Some(old), Some(new)) => (*old != new).then_some(Change::Changed),
                // Not on this computer: changed if it changed since it was last looked at.
                (Some(_), None) => seen.get(&f.path).is_some_and(|s| s.size != f.size || s.modified != f.modified).then_some(Change::Changed),
            };
            if let Some(change) = change {
                changes.push(FileChange { path: f.path.clone(), change, local: f.local });
            }
        }
        if !truncated {
            for path in kept.keys().filter(|p| !present.contains(p.as_str())) {
                changes.push(FileChange { path: path.clone(), change: Change::Deleted, local: true });
            }
        }
        seen.retain(|p, _| present.contains(p.as_str()));
        if seen != before {
            self.save_seen(&seen);
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Status { head: head.map(|c| c.id), changes, files: work.len(), truncated })
    }

    /// Saves the named files as they are in the folder now as a new version (a file no longer in
    /// the folder is removed from it). Files not named keep the version they had.
    pub fn check_in(&self, paths: &[String], message: &str, author: &Author, time: i64) -> Result<Commit> {
        let message = message.trim();
        if message.is_empty() {
            return Err(refused("say what changed: the history shows it"));
        }
        if paths.is_empty() {
            return Err(refused("name the parts to check in"));
        }
        if !self.exists() {
            self.start()?;
        }
        let (head, mut files) = self.head_files()?;
        let before = files.clone();
        let mut seen = self.load_seen();
        let mut notes = self.load_notes();
        let mut noted = Vec::new();
        for path in paths {
            check_path(path)?;
            let abs = self.work.join(path);
            if abs.is_file() {
                if !is_kept(path) {
                    return Err(refused(format!("{path} isn't a design or STEP file, which are all versions keep")));
                }
                let meta = std::fs::metadata(&abs).map_err(|e| io(path, e))?;
                let id = self.write_blob_from(&abs)?;
                // Read the date after the file was read, so a save meanwhile shows as a change.
                let modified = std::fs::metadata(&abs).map(|m| modified_ns(&m)).unwrap_or(0);
                seen.insert(path.clone(), Seen { size: meta.len(), modified, id: id.clone() });
                files.insert(path.clone(), id);
            } else if files.remove(path).is_none() {
                return Err(refused(format!("{path} isn't in the folder or its versions")));
            }
            if let Some(note) = notes.remove(path) {
                noted.push(note);
            }
        }
        if files == before {
            return Err(refused("nothing changed in those parts since the last version"));
        }
        let mut text = message.to_string();
        if !noted.is_empty() {
            text.push_str("\n\n");
            text.push_str(&noted.join("\n"));
        }
        let tree = self.write_tree(&files)?;
        let parent = head.as_ref().map(|c| c.id.as_str());
        let id = self.write_commit(&tree, parent, author, time, &text)?;
        write_replace(&self.git.join(self.head_ref()), format!("{id}\n").as_bytes())?;
        self.save_seen(&seen);
        self.save_notes(&notes);
        self.read_commit(&id)
    }

    /// Versions, newest first, with the files each changed; with `path`, only those that changed
    /// that file. Returns at most `limit`, and whether there are more.
    pub fn log(&self, path: Option<&str>, limit: usize) -> Result<(Vec<LogEntry>, bool)> {
        let mut out = Vec::new();
        let mut next = self.head()?;
        let mut walked = 0usize;
        while let Some(id) = next {
            walked += 1;
            if walked > MAX_WALK {
                break;
            }
            let c = self.read_commit(&id)?;
            let parent = match c.parents.first() {
                Some(p) => Some(self.read_commit(p)?),
                None => None,
            };
            let mut files = Vec::new();
            self.diff_trees(parent.as_ref().map(|p| p.tree.as_str()), Some(&c.tree), "", &mut files, 0)?;
            files.sort_by(|a, b| a.0.cmp(&b.0));
            next = c.parents.first().cloned();
            if path.is_none_or(|p| files.iter().any(|(f, _)| f == p)) {
                if out.len() == limit {
                    return Ok((out, true));
                }
                out.push(LogEntry { commit: c, files });
            }
        }
        Ok((out, false))
    }

    /// The stored id of `path` in version `commit`.
    pub fn file_in(&self, commit: &str, path: &str) -> Result<Option<String>> {
        check_path(path)?;
        let c = self.read_commit(commit)?;
        self.find(&c.tree, path)
    }

    /// Puts files back as they were in version `commit`: one file, or (no `path`) every kept
    /// file that differs. Files added since stay. The folder then shows them as changes, to
    /// check in like any other. Returns the files written.
    pub fn restore(&self, commit: &str, path: Option<&str>) -> Result<Vec<String>> {
        let c = self.read_commit(commit)?;
        let wanted: Vec<(String, String)> = match path {
            Some(p) => {
                let id = self.file_in(commit, p)?.ok_or_else(|| refused(format!("{p} wasn't in version {}", short(commit))))?;
                vec![(p.to_string(), id)]
            }
            None => self.flatten(&c.tree)?.into_iter().collect(),
        };
        let mut seen = self.load_seen();
        let mut written = Vec::new();
        for (p, id) in wanted {
            check_path(&p)?;
            let abs = self.work.join(&p);
            let same = std::fs::metadata(&abs).ok().is_some_and(|m| {
                seen.get(&p).is_some_and(|s| s.size == m.len() && s.modified == modified_ns(&m) && s.id == id)
                    || (is_local(&m) && Store::hash_file(&abs).is_ok_and(|h| h == id))
            });
            if same {
                continue;
            }
            self.copy_blob(&id, &abs)?;
            if let Ok(m) = std::fs::metadata(&abs) {
                seen.insert(p.clone(), Seen { size: m.len(), modified: modified_ns(&m), id });
            }
            written.push(p);
        }
        self.save_seen(&seen);
        Ok(written)
    }

    /// Writes `path` as it was in version `commit` to `dest` (a file that must not exist yet).
    /// With `note`, the next check-in of `dest` (relative to the folder) records it, such as
    /// "Copied from Frame plate (3a41f9e)".
    pub fn copy_out(&self, commit: &str, path: &str, dest: &Path, note: Option<(&str, &str)>) -> Result<()> {
        let id = self.file_in(commit, path)?.ok_or_else(|| refused(format!("{path} wasn't in version {}", short(commit))))?;
        if dest.exists() {
            return Err(refused(format!("{} already exists", dest.display())));
        }
        self.copy_blob(&id, dest)?;
        if let Some((rel, text)) = note {
            let mut notes = self.load_notes();
            notes.insert(rel.to_string(), text.to_string());
            self.save_notes(&notes);
        }
        Ok(())
    }

    fn load_notes(&self) -> BTreeMap<String, String> {
        std::fs::read(self.git.join(NOTES)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save_notes(&self, notes: &BTreeMap<String, String>) {
        let path = self.git.join(NOTES);
        if notes.is_empty() {
            let _ = std::fs::remove_file(path);
        } else if let Ok(bytes) = serde_json::to_vec(notes) {
            let _ = write_replace(&path, &bytes);
        }
    }

    // ---- names ----

    /// Names version `commit`: a version of the whole folder (`version`, such as "v1.3"), or a
    /// part's revision (`part` and `revision`, such as "Rev B"). Names never move once given.
    pub fn name(&self, commit: &str, version: Option<&str>, part: Option<(&str, &str)>, author: &Author, time: i64) -> Result<Tag> {
        let c = self.read_commit(commit)?;
        let note = match (version.map(str::trim).filter(|v| !v.is_empty()), part) {
            (Some(v), None) => TagNote { version: Some(v.chars().take(80).collect()), ..Default::default() },
            (None, Some((p, r))) => {
                check_path(p)?;
                let r = r.trim();
                if r.is_empty() {
                    return Err(refused("name the revision, such as Rev B"));
                }
                if self.find(&c.tree, p)?.is_none() {
                    return Err(refused(format!("{p} wasn't in version {}", short(commit))));
                }
                if self.tags()?.iter().any(|t| t.part.as_deref() == Some(p) && t.revision.as_deref() == Some(r)) {
                    return Err(refused(format!("{p} already has a {r}")));
                }
                TagNote { part: Some(p.to_string()), revision: Some(r.chars().take(80).collect()), ..Default::default() }
            }
            _ => return Err(refused("give either a version name, or a part and its revision")),
        };
        let base = match (&note.version, &note.part, &note.revision) {
            (Some(v), _, _) => ref_name(v),
            (None, Some(p), Some(r)) => {
                let digest = hex(&Sha1::digest(p.as_bytes()));
                let stem = Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                format!("{}-{}-{}", ref_name(&stem), digest.get(..6).unwrap_or(""), ref_name(r))
            }
            _ => return Err(refused("give either a version name, or a part and its revision")),
        };
        let tags = self.git.join("refs/tags");
        let mut name = base.clone();
        let mut n = 2;
        while tags.join(&name).exists() {
            if note.version.is_some() && n == 2 {
                return Err(refused(format!("there's already a version called {}", note.version.as_deref().unwrap_or(&base))));
            }
            name = format!("{base}-{n}");
            n += 1;
        }
        let body = serde_json::to_string(&note).map_err(|e| damaged(e.to_string()))?;
        let text = format!("object {}\ntype commit\ntag {name}\ntagger {} <{}> {time} +0000\n\n{body}\n", c.id, author.name, author.email);
        let id = self.write_object("tag", text.as_bytes())?;
        write_replace(&tags.join(&name), format!("{id}\n").as_bytes())?;
        Ok(Tag { name, commit: c.id, version: note.version, part: note.part, revision: note.revision })
    }

    /// Every named version. A plain git tag counts as a version named after it.
    pub fn tags(&self) -> Result<Vec<Tag>> {
        let dir = self.git.join("refs/tags");
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(out) };
        for entry in entries.flatten().take(MAX_FILES) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(Some(id)) = self.read_ref(&format!("refs/tags/{name}")) else { continue };
            let Ok(kind) = self.kind_of(&id) else { continue };
            if kind == "commit" {
                out.push(Tag { name: name.clone(), commit: id, version: Some(name), part: None, revision: None });
                continue;
            }
            let Ok(data) = self.read_small(&id, "tag") else { continue };
            let text = String::from_utf8_lossy(&data);
            let (headers, body) = text.split_once("\n\n").unwrap_or((&text, ""));
            let Some(commit) = headers.lines().find_map(|l| l.strip_prefix("object ")).map(str::trim).filter(|i| is_id(i)) else { continue };
            let note: TagNote = serde_json::from_str(body.trim()).unwrap_or_else(|_| TagNote { version: Some(name.clone()), ..Default::default() });
            out.push(Tag { name, commit: commit.to_string(), version: note.version, part: note.part, revision: note.revision });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

#[cfg(test)]
#[path = "versions_tests.rs"]
mod tests;
