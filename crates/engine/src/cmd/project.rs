//! The project folder, for the assistant (docs/agent-chat.md, "The project folder"):
//! `project.files` lists it, `project.search` looks through its file names and text, and
//! `project.read` reads one file: text as it is, a PDF's or Word file's text, a KiCad schematic or
//! board as a summary. They only read, and only inside the project ([`crate::project`]), and only
//! while the user allows it. `file.text` reads a PDF or Word file anywhere: project.read runs it
//! in the helper process, so a damaged PDF can't take SolidCraft down.

use serde_json::Value;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::project::{
    FileTextParams, FileTextReply, ProjectFileEntry, ProjectFilesParams, ProjectFilesReply, ProjectHit, ProjectReadParams, ProjectReadReply,
    ProjectSearchParams, ProjectSearchReply,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};
use solidcraft_io::project as files;

const FILES_CONTRACT: TypedContract<ProjectFilesParams> = TypedContract::new();
const SEARCH_CONTRACT: TypedContract<ProjectSearchParams> = TypedContract::new();
const READ_CONTRACT: TypedContract<ProjectReadParams> = TypedContract::new();
const TEXT_CONTRACT: TypedContract<FileTextParams> = TypedContract::new();

/// Defaults when a call names no limit.
const DEFAULT_FILES: usize = 500;
const DEFAULT_MATCHES: usize = 100;
const DEFAULT_CHARS: usize = 20_000;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("project.files", "List Project Files", crate::checked!("project.files", list))
        .icon("open")
        .noundo()
        .contract(&FILES_CONTRACT)
        .output(output_schema_for::<ProjectFilesReply>)
        .params(
            "path?: a folder in the project (default all of it); filter?: text in the name, or a kind (design, cad, mesh, pdf, kicad, \
             text, word, office, picture); limit?: most files (default 500, 1 to 5000) → project, folder, brief? (PROJECT.md or \
             README.md: read it first), files: [{path, kind, bytes, modified}], total, truncated. The project is the folder that \
             keeps versions of the design in front, else the design's own folder; hidden files and build folders are left out. \
             Reads only; needs Preferences → AI → project folder on",
        ),
    CommandSpec::new("project.search", "Search Project", crate::checked!("project.search", search))
        .icon("open")
        .noundo()
        .contract(&SEARCH_CONTRACT)
        .output(output_schema_for::<ProjectSearchReply>)
        .params(
            "query: text to look for, any case (a part number, a reference such as J3, a net); path?: a folder in the project; \
             limit?: most matches (default 100) → matches: [{path, line?, text}] (file names, then lines of text and KiCad \
             files), files_searched, truncated. PDFs are not searched: read them with project.read",
        ),
    CommandSpec::new("project.read", "Read Project File", crate::checked!("project.read", read))
        .icon("open")
        .noundo()
        .contract(&READ_CONTRACT)
        .output(output_schema_for::<ProjectReadReply>)
        .params(
            "path: a file as project.files lists it; offset?: characters to skip (a reply's next_offset); max_chars?: default \
             20000, 1000 to 100000; raw?: a KiCad schematic or board as its own text → path, kind, bytes, modified, summary? (a \
             KiCad schematic's or board's parts with reference, value, symbol or footprint, fields and place; sheets, labels, nets, \
             layers, outline), text? (text files as they are, a PDF's text page by page, a Word file's paragraphs), pages?, offset, \
             next_offset? (more follows), note? (a design, CAD file or picture: what to do instead)",
        ),
    CommandSpec::new("file.text", "Read Document Text", crate::checked!("file.text", text))
        .icon("open")
        .noundo()
        .contract(&TEXT_CONTRACT)
        .output(output_schema_for::<FileTextReply>)
        .params("path: a .pdf or .docx file (a full path); offset?; max_chars? (default 20000) → path, text, pages?, offset, next_offset?"),
];

/// The project folder, or why the assistant can't read one.
fn project(s: &Session, cmd: &str) -> Result<std::path::PathBuf> {
    if !crate::project::allowed() {
        return Err(bad(cmd, "reading the project folder is off (Preferences → AI)"));
    }
    crate::project::folder(s.path.as_deref()).ok_or_else(|| {
        bad(cmd, "there is no project folder: save the design in its project's folder (the folder that keeps its versions, or its own)")
    })
}

/// The whole number `key` in `lo..=hi`, `default` when the call gives none, or why it can't be used.
fn number(cmd: &str, p: &Value, key: &str, lo: u64, hi: u64, default: usize) -> Result<usize> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_u64().filter(|n| (lo..=hi).contains(n)).map(|n| n as usize).ok_or_else(|| bad(cmd, format!("`{key}` must be {lo} to {hi}"))),
    }
}

fn to_value(cmd: &str, v: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| bad(cmd, e.to_string()))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "project.files";
    let folder = project(s, cmd)?;
    let under = p.get("path").and_then(Value::as_str).unwrap_or_default();
    let filter = p.get("filter").and_then(Value::as_str);
    let limit = number(cmd, p, "limit", 1, 5000, DEFAULT_FILES)?;
    let found = files::list(&folder, under, filter, limit).map_err(|e| bad(cmd, e))?;
    let reply = ProjectFilesReply {
        project: folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        folder: folder.to_string_lossy().into_owned(),
        brief: files::brief(&folder),
        files: found
            .files
            .into_iter()
            .map(|f| ProjectFileEntry { path: f.path, kind: f.kind.to_string(), bytes: f.bytes, modified: f.modified })
            .collect(),
        total: found.total as u64,
        truncated: found.truncated,
    };
    to_value(cmd, reply)
}

fn search(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "project.search";
    let folder = project(s, cmd)?;
    let query = p.get("query").and_then(Value::as_str).unwrap_or_default();
    let under = p.get("path").and_then(Value::as_str).unwrap_or_default();
    let limit = number(cmd, p, "limit", 1, 1000, DEFAULT_MATCHES)?;
    let found = files::search(&folder, under, query, limit).map_err(|e| bad(cmd, e))?;
    let reply = ProjectSearchReply {
        matches: found.matches.into_iter().map(|h| ProjectHit { path: h.path, line: h.line.map(|n| n as u64), text: h.text }).collect(),
        files_searched: found.files_searched as u64,
        truncated: found.truncated,
    };
    to_value(cmd, reply)
}

/// `text` from `offset`, at most `max` characters: the part, and where the next starts.
fn part(text: &str, offset: usize, max: usize) -> (String, Option<u64>) {
    let (out, more) = files::page(text, offset, max);
    let next = more.then(|| (offset + out.chars().count()) as u64);
    (out, next)
}

fn read(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "project.read";
    let folder = project(s, cmd)?;
    let rel = p.get("path").and_then(Value::as_str).unwrap_or_default();
    let offset = number(cmd, p, "offset", 0, u64::from(u32::MAX), 0)?;
    let max = number(cmd, p, "max_chars", 1000, 100_000, DEFAULT_CHARS)?;
    let raw = p.get("raw").and_then(Value::as_bool).unwrap_or(false);
    let path = files::resolve(&folder, rel).map_err(|e| bad(cmd, e))?;
    let meta = std::fs::metadata(&path).map_err(|e| bad(cmd, format!("{rel}: {e}")))?;
    if !meta.is_file() {
        return Err(bad(cmd, format!("`{rel}` is a folder: list it with project.files")));
    }
    let kind = files::kind(rel);
    let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let mut reply = ProjectReadReply {
        path: rel.trim_matches(['/', '\\']).replace('\\', "/"),
        kind: kind.to_string(),
        bytes: meta.len(),
        modified,
        summary: None,
        text: None,
        pages: None,
        offset: offset as u64,
        next_offset: None,
        note: None,
    };
    match kind {
        "pdf" | "word" => {
            if meta.len() > files::MAX_DOCUMENT_BYTES {
                return Err(bad(cmd, format!("`{rel}` is larger than SolidCraft reads ({} MB)", files::MAX_DOCUMENT_BYTES >> 20)));
            }
            let params = serde_json::json!({"path": path.to_string_lossy(), "offset": offset, "max_chars": max});
            let stop = std::sync::atomic::AtomicBool::new(false);
            let got = crate::helper::run(&[("file.text", params)], &stop).map_err(|f| {
                if f.crashed { bad(cmd, format!("`{rel}` couldn't be read: reading it stopped the reader")) } else { bad(cmd, f.message) }
            })?;
            reply.text = got.get("text").and_then(Value::as_str).map(str::to_string);
            reply.pages = got.get("pages").and_then(Value::as_u64);
            reply.next_offset = got.get("next_offset").and_then(Value::as_u64);
        }
        "kicad" | "text" => {
            if meta.len() > files::MAX_TEXT_BYTES {
                return Err(bad(cmd, format!("`{rel}` is larger than SolidCraft reads as text ({} MB)", files::MAX_TEXT_BYTES >> 20)));
            }
            let bytes = std::fs::read(&path).map_err(|e| bad(cmd, format!("{rel}: {e}")))?;
            let text = files::decode(&bytes);
            let summarised = kind == "kicad" && !raw && offset == 0;
            match summarised.then(|| solidcraft_io::kicad::summary(&text)) {
                Some(Ok(summary)) => reply.summary = Some(to_value(cmd, summary)?),
                // Project files, libraries and anything the summary doesn't cover: as text.
                _ => {
                    let (t, next) = part(&text, offset, max);
                    reply.text = Some(t);
                    reply.next_offset = next;
                }
            }
        }
        "design" => reply.note = Some("a SolidCraft design: open it with `open`, or put it in the open design with component.insert_design".into()),
        "cad" | "mesh" => reply.note = Some("a CAD or mesh file: open or insert it to look at it".into()),
        "picture" => reply.note = Some("a picture: this tool reads text only; ask the user to add it to the chat to see it".into()),
        _ => reply.note = Some("this kind of file isn't read here; a PDF, Word, text or KiCad copy of it can be".into()),
    }
    to_value(cmd, reply)
}

fn text(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.text";
    let path = p.get("path").and_then(Value::as_str).unwrap_or_default();
    if !std::path::Path::new(path).is_absolute() {
        return Err(bad(cmd, format!("`path` must be a full path, not {path}")));
    }
    let offset = number(cmd, p, "offset", 0, u64::from(u32::MAX), 0)?;
    let max = number(cmd, p, "max_chars", 1000, 100_000, DEFAULT_CHARS)?;
    let meta = std::fs::metadata(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    if meta.len() > files::MAX_DOCUMENT_BYTES {
        return Err(bad(cmd, format!("{path} is larger than SolidCraft reads")));
    }
    let bytes = std::fs::read(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    let (all, pages) = match files::kind(path) {
        "pdf" => files::pdf_text(&bytes).map(|(t, n)| (t, Some(n as u64))),
        "word" => files::docx_text(&bytes).map(|t| (t, None)),
        _ => Err("only a PDF or Word (.docx) file is read here".to_string()),
    }
    .map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    let (text, next_offset) = part(&all, offset, max);
    to_value(cmd, FileTextReply { path: path.to_string(), text, pages, offset: offset as u64, next_offset })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use serde_json::json;

    use crate::Session;

    /// A folder removed when the test ends.
    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A project with notes, a schematic, a Word file and a design saved in it.
    fn project() -> (Dir, Session) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("solidcraft-project-cmd-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Electrical")).unwrap();
        std::fs::write(dir.join("PROJECT.md"), "# Pod\nJ3 is the 48 V input.\n").unwrap();
        std::fs::write(dir.join("Electrical/notes.txt"), "a".repeat(2500)).unwrap();
        std::fs::write(dir.join("Electrical/photo.jpg"), b"\xff\xd8").unwrap();
        let s = Session { path: Some(dir.join("pod.sc").to_string_lossy().into_owned()), ..Default::default() };
        (Dir(dir), s)
    }

    #[test]
    fn the_assistant_lists_searches_and_reads_the_designs_project() {
        let (_dir, mut s) = project();
        let l = s.execute("project.files", &json!({})).unwrap();
        assert_eq!(l["brief"], "PROJECT.md");
        let paths: Vec<&str> = l["files"].as_array().unwrap().iter().filter_map(|f| f["path"].as_str()).collect();
        assert_eq!(paths, ["PROJECT.md", "Electrical/notes.txt", "Electrical/photo.jpg"]);
        let hits = s.execute("project.search", &json!({"query": "j3"})).unwrap();
        assert_eq!(hits["matches"][0]["path"], "PROJECT.md");
        assert_eq!(hits["matches"][0]["line"], 2);
        let first = s.execute("project.read", &json!({"path": "Electrical/notes.txt", "max_chars": 1000})).unwrap();
        assert_eq!(first["text"].as_str().unwrap().len(), 1000);
        assert_eq!(first["next_offset"], 1000);
        let last = s.execute("project.read", &json!({"path": "Electrical/notes.txt", "offset": 2000, "max_chars": 1000})).unwrap();
        assert_eq!(last["text"].as_str().unwrap().len(), 500);
        assert!(last.get("next_offset").is_none());
        let pic = s.execute("project.read", &json!({"path": "Electrical/photo.jpg"})).unwrap();
        assert!(pic["note"].as_str().unwrap().contains("picture"));
    }

    #[test]
    fn nothing_outside_the_project_is_read_and_no_project_says_why() {
        let (_dir, mut s) = project();
        for bad in ["../secret.txt", "/etc/passwd", ".git/config", "Electrical"] {
            assert!(s.execute("project.read", &json!({"path": bad})).is_err(), "{bad}");
        }
        assert!(s.execute("project.files", &json!({"path": "../.."})).is_err());
        assert!(s.execute("project.files", &json!({"limit": 0})).is_err());
        assert!(s.execute("file.text", &json!({"path": "relative.pdf"})).is_err());
        let mut unsaved = Session::default();
        let e = unsaved.execute("project.files", &json!({})).unwrap_err().to_string();
        assert!(e.contains("no project folder"), "{e}");
    }
}
