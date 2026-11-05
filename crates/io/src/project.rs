//! A project folder, as the assistant reads it (docs/agent-chat.md, "The project folder"): the
//! files in it, a search through their names and text, and one file's text: a document's words,
//! a PDF's text, a KiCad schematic or board as a summary ([`crate::kicad`]). Everything here only
//! reads, and only inside the folder: a path is checked part by part and then again after links
//! are followed, so neither `..` nor a link can lead out of it. Hidden files, version history and
//! build output are left out.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The most files a walk looks at before it stops, and how deep it goes.
const MAX_VISITED: usize = 200_000;
const MAX_DEPTH: usize = 32;
/// The largest file read as text, and the largest searched.
pub const MAX_TEXT_BYTES: u64 = 16 << 20;
const MAX_SEARCH_BYTES: u64 = 4 << 20;
/// The largest PDF or Word file read.
pub const MAX_DOCUMENT_BYTES: u64 = 64 << 20;
/// Text a Word file gives at most (its XML is read within this too).
const MAX_DOCX_XML_BYTES: u64 = 32 << 20;
/// Characters one search result line keeps.
const MAX_HIT_CHARS: usize = 240;
/// The files a project's notes for the assistant may be in, most wanted first.
pub const BRIEFS: [&str; 4] = ["PROJECT.md", "project.md", "README.md", "readme.md"];

/// Files read as text as they are.
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "csv", "tsv", "json", "yaml", "yml", "toml", "xml", "ini", "cfg", "conf", "log", "svg", "html", "htm", "dxf", "net",
    "cir", "sp", "spice", "rst", "adoc", "tex", "bom", "pos", "drl", "gbr", "py", "rs", "c", "h", "cpp", "ino",
];
const PICTURE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic"];
const OFFICE_EXTS: &[&str] = &["docx", "xlsx", "pptx", "doc", "xls", "ppt", "odt", "ods", "pages", "numbers", "key"];

/// What a file in a project is, by its name: `design`, `cad` (STEP, IGES and other CAD files),
/// `mesh`, `pdf`, `kicad`, `text`, `word` (a .docx), `office` (another office file), `picture`,
/// or `other`.
pub fn kind(path: &str) -> &'static str {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let is = |list: &[&str]| list.contains(&ext.as_str());
    if crate::is_design_path(path) {
        "design"
    } else if crate::kicad::is_kicad_path(path) || matches!(file_name(path), "fp-lib-table" | "sym-lib-table") {
        "kicad"
    } else if ext == "pdf" {
        "pdf"
    } else if ext == "docx" {
        "word"
    } else if is(TEXT_EXTS) {
        "text"
    } else if crate::is_step_path(path) {
        "cad"
    } else if crate::is_mesh_path(path) {
        "mesh"
    } else if is(PICTURE_EXTS) {
        "picture"
    } else if is(OFFICE_EXTS) {
        "office"
    } else {
        "other"
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A file in a project.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectFile {
    /// Its place in the project, `/` between folders.
    pub path: String,
    /// What it is ([`kind`]).
    pub kind: &'static str,
    pub bytes: u64,
    /// When it last changed, in seconds since 1970 (0 when the system doesn't say).
    pub modified: i64,
}

/// A walk through a project.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Listing {
    pub files: Vec<ProjectFile>,
    /// Files that matched, listed or not.
    pub total: usize,
    /// The walk stopped early (too many files or folders too deep) or the list was cut at its
    /// limit.
    pub truncated: bool,
}

/// `rel` checked as a place inside a project: relative, `/` or `\` between parts, none of them
/// empty, `.`, `..` or hidden. `""` is the project itself.
pub fn check_relative(rel: &str) -> Result<Vec<&str>, String> {
    let rel = rel.trim().trim_matches(['/', '\\']);
    if rel.is_empty() {
        return Ok(Vec::new());
    }
    let parts: Vec<&str> = rel.split(['/', '\\']).collect();
    let bad = rel.len() > 4096
        || rel.contains(['\0', '\n', '\r'])
        || rel.contains(':')
        || parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.starts_with('.'));
    if bad { Err(format!("`{rel}` isn't a place inside the project")) } else { Ok(parts) }
}

/// `rel` inside `folder`, or why not: it must name a place inside the folder that exists, and stay
/// inside it once links are followed.
pub fn resolve(folder: &Path, rel: &str) -> Result<PathBuf, String> {
    let parts = check_relative(rel)?;
    let root = folder.canonicalize().map_err(|e| format!("the project folder {} can't be read: {e}", folder.display()))?;
    let mut path = root.clone();
    for p in parts {
        path.push(p);
    }
    let real = path.canonicalize().map_err(|_| format!("`{rel}` isn't in the project"))?;
    if !real.starts_with(&root) {
        return Err(format!("`{rel}` leads out of the project"));
    }
    Ok(real)
}

/// `path` relative to `root`, `/` between folders.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Every file under `start` (inside `root`), in a stable order (folders by name, depth first),
/// passed to `each` until it returns false. What a folder's versions leave out is left out here
/// too (hidden, lock, backup and temporary files, build folders: [`crate::versions::is_kept`]), and
/// links. True when the walk stopped early.
fn walk(root: &Path, start: &Path, each: &mut dyn FnMut(&Path, &std::fs::Metadata) -> bool) -> bool {
    let mut stack = vec![(start.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<(String, PathBuf)> = read.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
        entries.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then(a.0.cmp(&b.0)));
        let mut folders = Vec::new();
        for (name, path) in entries {
            visited += 1;
            if visited > MAX_VISITED {
                return true;
            }
            if name.starts_with('.') {
                continue;
            }
            // Never a link: it could lead out of the project.
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                if crate::versions::is_skipped_folder(&name) {
                    continue;
                }
                if depth + 1 > MAX_DEPTH {
                    return true;
                }
                folders.push((path, depth + 1));
            } else if meta.is_file() && crate::versions::is_kept(&name) && relative(root, &path).is_some() && !each(&path, &meta) {
                return true;
            }
        }
        // Depth first, in name order.
        stack.extend(folders.into_iter().rev());
    }
    false
}

fn modified(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The files in `folder` (or its sub-folder `under`) whose name or kind matches `filter` (text in
/// the name, any case, or a kind such as `pdf`; `None` for all), at most `limit` of them.
pub fn list(folder: &Path, under: &str, filter: Option<&str>, limit: usize) -> Result<Listing, String> {
    let root = resolve(folder, "")?;
    let start = resolve(folder, under)?;
    if !start.is_dir() {
        return Err(format!("`{under}` isn't a folder in the project"));
    }
    let filter = filter.map(|f| f.trim().trim_start_matches('.').to_lowercase()).filter(|f| !f.is_empty());
    let mut out = Listing::default();
    let stopped = walk(&root, &start, &mut |path, meta| {
        let Some(rel) = relative(&root, path) else { return true };
        let kind = kind(&rel);
        let matches = filter.as_deref().is_none_or(|f| kind == f || file_name(&rel).to_lowercase().contains(f));
        if matches {
            out.total += 1;
            if out.files.len() < limit {
                out.files.push(ProjectFile { path: rel, kind, bytes: meta.len(), modified: modified(meta) });
            }
        }
        true
    });
    out.truncated = stopped || out.total > out.files.len();
    Ok(out)
}

/// The first of [`BRIEFS`] at the top of `folder`.
pub fn brief(folder: &Path) -> Option<String> {
    BRIEFS.iter().find(|b| folder.join(b).is_file()).map(|b| (*b).to_string())
}

/// One place a search found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hit {
    pub path: String,
    /// The line it is on, from 1; none when the file's name matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// The line, cut short; or the file's kind when its name matched.
    pub text: String,
}

/// What a search found.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Found {
    pub matches: Vec<Hit>,
    /// Text files looked through.
    pub files_searched: usize,
    /// There were more matches than `limit`, or the walk stopped early.
    pub truncated: bool,
}

/// The files under `under` in `folder` whose name holds `query` (any case), and the lines of its
/// text files (KiCad files included, PDFs not) that do, at most `limit` matches.
pub fn search(folder: &Path, under: &str, query: &str, limit: usize) -> Result<Found, String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Err("give the text to look for".into());
    }
    let root = resolve(folder, "")?;
    let start = resolve(folder, under)?;
    let mut out = Found::default();
    let stopped = walk(&root, &start, &mut |path, meta| {
        let Some(rel) = relative(&root, path) else { return true };
        let kind = kind(&rel);
        if file_name(&rel).to_lowercase().contains(&query) {
            if out.matches.len() >= limit {
                out.truncated = true;
                return false;
            }
            out.matches.push(Hit { path: rel.clone(), line: None, text: kind.to_string() });
        }
        if !matches!(kind, "text" | "kicad") || meta.len() > MAX_SEARCH_BYTES {
            return true;
        }
        let Ok(bytes) = std::fs::read(path) else { return true };
        out.files_searched += 1;
        for (n, line) in decode(&bytes).lines().enumerate() {
            if line.to_lowercase().contains(&query) {
                if out.matches.len() >= limit {
                    out.truncated = true;
                    return false;
                }
                out.matches.push(Hit { path: rel.clone(), line: Some(n + 1), text: line.trim().chars().take(MAX_HIT_CHARS).collect() });
            }
        }
        true
    });
    out.truncated |= stopped;
    Ok(out)
}

/// Bytes as text: UTF-8 as it is (a byte-order mark dropped), else the encoding detected.
pub fn decode(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.trim_start_matches('\u{feff}').to_owned();
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes.get(..bytes.len().min(64 << 10)).unwrap_or_default(), true);
    detector.guess(None, false).decode_with_bom_removal(bytes).0.into_owned()
}

/// The text of a PDF, one page after another, each page starting with a `--- page N ---` line.
pub fn pdf_text(bytes: &[u8]) -> Result<(String, usize), String> {
    if !bytes.starts_with(b"%PDF") && !bytes.get(..1024).unwrap_or(bytes).windows(4).any(|w| w == b"%PDF") {
        return Err("this isn't a PDF file".into());
    }
    let pages = pdf_extract::extract_text_from_mem_by_pages(bytes).map_err(|e| format!("the PDF can't be read: {e}"))?;
    let mut out = String::new();
    for (i, page) in pages.iter().enumerate() {
        out.push_str(&format!("--- page {} ---\n", i + 1));
        out.push_str(tidy(page).trim());
        out.push_str("\n\n");
    }
    Ok((out, pages.len()))
}

/// Runs of blank lines and trailing spaces taken out of extracted text.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The text of a Word (.docx) file: its paragraphs, one per line, tables' cells separated by tabs.
pub fn docx_text(bytes: &[u8]) -> Result<String, String> {
    use quick_xml::events::Event;
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("this isn't a Word file SolidCraft can read: {e}"))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| "this Word file has no document in it".to_string())?
        .take(MAX_DOCX_XML_BYTES)
        .read_to_string(&mut xml)
        .map_err(|e| format!("the Word file can't be read: {e}"))?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"t" => in_text = true,
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => out.push('\n'),
                b"tc" => out.push('\t'),
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                b"tab" => out.push('\t'),
                b"br" | b"cr" => out.push('\n'),
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => out.push_str(&t.decode().map_err(|e| format!("the Word file is damaged: {e}"))?),
            Ok(Event::GeneralRef(r)) if in_text => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    out.push(c);
                } else {
                    let name = String::from_utf8_lossy(r.as_ref()).into_owned();
                    out.push_str(match name.as_str() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("the Word file is damaged: {e}")),
            _ => {}
        }
    }
    Ok(tidy(&out))
}

/// A slice of `text` by characters: from `offset`, at most `max` of them, and whether more follows.
pub fn page(text: &str, offset: usize, max: usize) -> (String, bool) {
    let mut chars = text.chars().skip(offset);
    let out: String = chars.by_ref().take(max).collect();
    (out, chars.next().is_some())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::io::Write;

    /// A folder removed when the test ends.
    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("solidcraft-project-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A one-page PDF showing `text` in Helvetica.
    pub(crate) fn tiny_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    fn project() -> Dir {
        let dir = Dir::new();
        let p = dir.path();
        for f in ["Electrical", "CAD/Parts", ".git", "node_modules/x", "Board-backups"] {
            std::fs::create_dir_all(p.join(f)).unwrap();
        }
        std::fs::write(p.join("PROJECT.md"), "# Thruster pod\nConnector J3 carries 48 V.\n").unwrap();
        std::fs::write(p.join("Electrical/pod.kicad_sch"), "(kicad_sch (version 20231120))\n(property \"Reference\" \"J3\")\n").unwrap();
        std::fs::write(p.join("Electrical/wiring.pdf"), b"%PDF-1.4 not really").unwrap();
        std::fs::write(p.join("CAD/Parts/bracket.sc"), b"x").unwrap();
        std::fs::write(p.join("CAD/Parts/bracket.step"), b"ISO-10303-21;").unwrap();
        std::fs::write(p.join(".git/config"), b"secret").unwrap();
        std::fs::write(p.join("node_modules/x/a.txt"), b"J3").unwrap();
        std::fs::write(p.join("Board-backups/old.kicad_sch"), b"J3").unwrap();
        std::fs::write(p.join("Electrical/.hidden.txt"), b"J3").unwrap();
        std::fs::write(p.join("CAD/Parts/bracket.sc.bak"), b"J3").unwrap();
        std::fs::write(p.join("Electrical/~$notes.docx"), b"J3").unwrap();
        dir
    }

    #[test]
    fn a_project_lists_its_files_by_kind_without_hidden_or_build_folders() {
        let dir = project();
        let all = list(dir.path(), "", None, 100).unwrap();
        let paths: Vec<(&str, &str)> = all.files.iter().map(|f| (f.path.as_str(), f.kind)).collect();
        assert_eq!(
            paths,
            [
                ("PROJECT.md", "text"),
                ("CAD/Parts/bracket.sc", "design"),
                ("CAD/Parts/bracket.step", "cad"),
                ("Electrical/pod.kicad_sch", "kicad"),
                ("Electrical/wiring.pdf", "pdf"),
            ]
        );
        assert!(!all.truncated);
        let pdfs = list(dir.path(), "Electrical", Some("pdf"), 100).unwrap();
        assert_eq!(pdfs.files.len(), 1);
        let cut = list(dir.path(), "", None, 2).unwrap();
        assert!(cut.truncated && cut.total == 5 && cut.files.len() == 2);
        assert_eq!(brief(dir.path()).as_deref(), Some("PROJECT.md"));
    }

    #[test]
    fn nothing_outside_the_project_or_hidden_in_it_is_reachable() {
        let dir = project();
        for bad in ["../x", "/etc/passwd", "C:\\Windows", ".git/config", "Electrical/.hidden.txt", "a//b", "missing.txt"] {
            assert!(resolve(dir.path(), bad).is_err(), "{bad}");
        }
        assert!(resolve(dir.path(), "Electrical/pod.kicad_sch").is_ok());
        assert!(resolve(dir.path(), "Electrical\\pod.kicad_sch").is_ok());
        #[cfg(unix)]
        {
            let outside = Dir::new();
            std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
            assert!(resolve(dir.path(), "link/secret.txt").is_err(), "a link out of the project is refused");
            assert!(list(dir.path(), "", Some("secret"), 10).unwrap().files.is_empty(), "links are never walked");
        }
    }

    #[test]
    fn a_search_finds_names_and_lines_in_text_and_kicad_files() {
        let dir = project();
        let found = search(dir.path(), "", "j3", 50).unwrap();
        let at: Vec<(&str, Option<usize>)> = found.matches.iter().map(|h| (h.path.as_str(), h.line)).collect();
        assert_eq!(at, [("PROJECT.md", Some(2)), ("Electrical/pod.kicad_sch", Some(2))]);
        let names = search(dir.path(), "", "bracket", 50).unwrap();
        assert_eq!(names.matches.len(), 2);
        assert!(names.matches.iter().all(|h| h.line.is_none()));
        assert!(search(dir.path(), "", "j3", 1).unwrap().truncated);
        assert!(search(dir.path(), "", "  ", 10).is_err());
    }

    #[test]
    fn a_word_file_gives_its_paragraphs_and_a_bad_one_says_so() {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut out);
        w.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(
            br#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>Pod wiring</w:t></w:r></w:p><w:p><w:r><w:t>J3 &amp; J4</w:t><w:tab/><w:t>48 V</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .unwrap();
        w.finish().unwrap();
        assert_eq!(docx_text(&out.into_inner()).unwrap(), "Pod wiring\nJ3 & J4\t48 V\n");
        assert!(docx_text(b"not a zip").is_err());
    }

    #[test]
    fn a_pdf_gives_its_text_page_by_page_and_one_that_isnt_is_refused() {
        let (text, pages) = pdf_text(&tiny_pdf("Pod wiring J3")).unwrap();
        assert_eq!(pages, 1);
        assert!(text.starts_with("--- page 1 ---\n") && text.contains("Pod wiring J3"), "{text:?}");
        assert!(pdf_text(b"hello").is_err());
        assert!(pdf_text(b"%PDF-1.4 broken").is_err());
        assert_eq!(page("abcdé", 1, 3), ("bcd".to_string(), true));
        assert_eq!(page("abcdé", 3, 10), ("dé".to_string(), false));
        assert_eq!(decode(b"\xef\xbb\xbfhi"), "hi");
    }
}
