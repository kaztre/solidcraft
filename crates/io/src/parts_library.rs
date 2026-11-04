//! Parts libraries: a folder tree of designs and CAD files that the Parts Library window and
//! `library.parts.search` list and search (docs/parts-library.md). Sub-folders are the library's
//! categories. A CAD file is listed from its name and size alone, so a library in a synced folder
//! (iCloud Drive, Google Drive, OneDrive) isn't downloaded to be listed; a design's part number,
//! description and revision come from its document part, without its blobs or geometry.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{IoError, Result};

/// The most parts one library lists; a larger folder is cut short (`truncated`).
pub const MAX_ENTRIES: usize = 50_000;
/// Folders and files looked at, parts or not, before the scan stops.
const MAX_VISITED: usize = 500_000;
/// How deep the scan goes below the library folder.
const MAX_DEPTH: usize = 32;
/// Longest property text kept for a part.
const MAX_TEXT: usize = 256;

/// What a library file is, which decides how it goes into a design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A SolidCraft design: inserted as a linked component (`component.insert_design`).
    Design,
    /// STEP, IGES or another CAD file SolidCraft reads: its bodies are copied in
    /// (`file.insert_step`).
    Cad,
}

/// One part of a library.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    /// Relative to the library folder, `/` between folders.
    pub path: String,
    /// The file name without its extension.
    pub name: String,
    /// The folders it is in, relative to the library folder, `/` between them ("" at the top).
    pub folder: String,
    pub kind: Kind,
    /// The file's extension, lower-case.
    pub format: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// The file's contents are on this computer: false for a file a synced folder (iCloud Drive)
    /// keeps only in the cloud until it is opened, which reading would download.
    pub local: bool,
}

/// A scanned library folder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    /// The folder, as given.
    pub root: String,
    /// Sorted by folder, then name.
    pub entries: Vec<Entry>,
    /// The folder held more than [`MAX_ENTRIES`] parts or [`MAX_VISITED`] files.
    pub truncated: bool,
}

impl Library {
    /// The file of `entry` on this computer.
    pub fn absolute(&self, entry: &Entry) -> PathBuf {
        entry.path.split('/').fold(PathBuf::from(&self.root), |p, part| p.join(part))
    }
}

/// Which of a part's files a folder lists when it has several with one name (a vendor's
/// `T200.STEP`, `T200.IGS` and `T200.STL`): the design, then STEP, then IGES, then the others.
/// `None`: not a library file (meshes, drawings, SolidWorks parts).
fn rank(ext: &str) -> Option<(Kind, u8)> {
    match ext {
        // A design under any extension a design has had: `.sc`, and `.solidcraft` parts converted
        // before designs moved to `.sc`.
        e if crate::DESIGN_EXTS.contains(&e) => Some((Kind::Design, 0)),
        "step" | "stp" => Some((Kind::Cad, 1)),
        "igs" | "iges" => Some((Kind::Cad, 2)),
        e if crate::NATIVE_CAD_EXTS.contains(&e) => Some((Kind::Cad, 3)),
        _ => None,
    }
}

fn text(metadata: &Map<String, Value>, key: &str) -> Option<String> {
    let t = metadata.get(key)?.as_str()?.trim();
    (!t.is_empty()).then(|| t.chars().take(MAX_TEXT).collect())
}

/// List the parts under `root`. Hidden files and folders (a leading `.`) and symbolic links are
/// skipped, and a sub-folder that can't be read is left out; only an unreadable `root` fails.
pub fn scan(root: &str) -> Result<Library> {
    // The browser has no folders to list: `is_dir` is false there, so a scan is refused.
    crate::vfs::check(root).map_err(|e| IoError::Invalid(format!("{root}: {e}")))?;
    let base = Path::new(root);
    if !base.is_dir() {
        return Err(IoError::Invalid(format!("{root} is not a folder")));
    }
    let mut library = Library { root: root.to_string(), ..Library::default() };
    let mut visited = 0usize;
    let mut folders = vec![(base.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, rel, depth)) = folders.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            if depth == 0 {
                return Err(IoError::Invalid(format!("can't read {root}")));
            }
            continue;
        };
        // This folder's parts by lower-case name: the best-ranked file of each.
        let mut here: std::collections::BTreeMap<String, (u8, Entry)> = Default::default();
        for item in read.flatten() {
            visited += 1;
            if visited > MAX_VISITED {
                library.truncated = true;
                break;
            }
            let Ok(file_type) = item.file_type() else { continue };
            let file_name = item.file_name();
            let Some(name) = file_name.to_str() else { continue };
            if name.starts_with('.') || file_type.is_symlink() {
                continue;
            }
            let inner = if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };
            if file_type.is_dir() {
                if depth < MAX_DEPTH {
                    folders.push((item.path(), inner, depth + 1));
                }
                continue;
            }
            let path = Path::new(name);
            let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|e| e.to_str())) else {
                continue;
            };
            let ext = ext.to_ascii_lowercase();
            let Some((kind, order)) = rank(&ext) else { continue };
            if here.get(&stem.to_lowercase()).is_some_and(|(best, _)| *best <= order) {
                continue;
            }
            let meta = item.metadata().ok();
            let bytes = meta.as_ref().map_or(0, |m| m.len());
            let local = meta.as_ref().is_none_or(is_local);
            let entry = Entry {
                path: inner,
                name: stem.to_string(),
                folder: rel.clone(),
                kind,
                format: ext,
                bytes,
                part_number: None,
                description: None,
                revision: None,
                local,
            };
            here.insert(stem.to_lowercase(), (order, entry));
        }
        let alone = here.len() == 1;
        for (_, (_, mut entry)) in here {
            if alone {
                name_after_folder(&mut entry);
            }
            if library.entries.len() >= MAX_ENTRIES {
                library.truncated = true;
                break;
            }
            if entry.kind == Kind::Design
                && let Some(metadata) = design_properties(&dir.join(Path::new(&entry.path).file_name().unwrap_or_default()))
            {
                entry.part_number = text(&metadata, "part_number");
                entry.description = text(&metadata, "description");
                entry.revision = text(&metadata, "revision");
            }
            library.entries.push(entry);
        }
        if library.truncated {
            break;
        }
    }
    library.entries.sort_by_cached_key(|e| (e.folder.to_lowercase(), e.name.to_lowercase()));
    Ok(library)
}

/// The words of a name, for comparing a folder's name with a file's.
fn words(text: &str) -> std::collections::BTreeSet<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 2).map(str::to_string).collect()
}

/// A part alone in a folder that is named after it (a vendor's download,
/// `M10_TO_M14_BULKHEAD_ADAPTER_BR-102909/BR-102512_RevA.STEP`) takes the folder's name, `_` as
/// spaces, and the folders above it as its category; a folder sharing no word with the file (a
/// category such as `THRUSTERS` holding one part) keeps the file's name.
fn name_after_folder(entry: &mut Entry) {
    let Some((parent, last)) = entry.folder.rsplit_once('/').or_else(|| (!entry.folder.is_empty()).then_some(("", entry.folder.as_str()))) else {
        return;
    };
    if words(last).is_disjoint(&words(&entry.name)) {
        return;
    }
    let name = last.replace('_', " ").trim().to_string();
    // A download unpacked into a folder of the same name (`X/X/x.step`) drops both.
    let mut folder = parent.to_string();
    while let Some(tail) = folder.rsplit('/').next().filter(|t| !t.is_empty() && t.replace('_', " ").eq_ignore_ascii_case(&name)) {
        let len = folder.len() - tail.len();
        folder.truncate(len.saturating_sub(1));
    }
    entry.name = name;
    entry.folder = folder;
}

/// A design file's properties, or `None` when it can't be read as a design.
fn design_properties(path: &Path) -> Option<Map<String, Value>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > crate::MAX_DESIGN_BYTES as u64 {
        return None;
    }
    crate::DesignContainerReader::new(file).ok()?.properties()
}

/// Whether a file's contents are on this computer. macOS marks a file that iCloud Drive keeps
/// only in the cloud as dataless; elsewhere every file counts as here.
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

/// A vendor part number in a part's name or file name, such as `BR-102909` in
/// `M10_TO_M14_BULKHEAD_ADAPTER_BR-102909`: one to five letters, a hyphen and at least three
/// digits, the first one found (the name before the file name).
pub fn guess_part_number(entry: &Entry) -> Option<String> {
    let file = entry.path.rsplit('/').next().unwrap_or("");
    [entry.name.as_str(), file].into_iter().find_map(|text| {
        let b = text.as_bytes();
        (0..b.len()).find_map(|start| {
            // A code starts a word: not straight after another letter or digit.
            if start > 0 && b[start - 1].is_ascii_alphanumeric() {
                return None;
            }
            let letters = b[start..].iter().take_while(|c| c.is_ascii_alphabetic()).count();
            let digits_at = start + letters + 1;
            if !(1..=5).contains(&letters) || b.get(start + letters) != Some(&b'-') {
                return None;
            }
            let digits = b.get(digits_at..)?.iter().take_while(|c| c.is_ascii_digit()).count();
            let end = digits_at + digits;
            (digits >= 3 && b.get(end).is_none_or(|c| !c.is_ascii_alphanumeric())).then(|| text[start..end].to_ascii_uppercase())
        })
    })
}

/// Lower-case, with `_` and `/` as spaces, so "t200 thruster" finds `T200_THRUSTER`.
fn fold(text: &str) -> String {
    text.to_lowercase().replace(['_', '/'], " ")
}

/// The words of a search, folded as the parts' text is.
pub fn terms(query: &str) -> Vec<String> {
    fold(query).split_whitespace().take(16).map(str::to_string).collect()
}

/// How well `entry` matches every one of `terms` (more: a better match), or `None` when one of
/// them is in none of its name, folders, file type, part number and description.
pub fn score(entry: &Entry, terms: &[String]) -> Option<usize> {
    let name = fold(&entry.name);
    let number = entry.part_number.as_deref().map(fold).unwrap_or_default();
    let rest = fold(&format!("{} {} {}", entry.path, entry.format, entry.description.as_deref().unwrap_or("")));
    terms.iter().try_fold(0, |score, term| {
        if name.contains(term.as_str()) || number.contains(term.as_str()) { Some(score + 1) } else { rest.contains(term.as_str()).then_some(score) }
    })
}

/// The parts that match `query` (all of them for a blank one), best first, then in library
/// order, at most `limit`; and how many matched.
pub fn search<'a>(library: &'a Library, query: &str, limit: usize) -> (Vec<&'a Entry>, usize) {
    let terms = terms(query);
    let mut found: Vec<(usize, &Entry)> = library.entries.iter().filter_map(|e| score(e, &terms).map(|s| (s, e))).collect();
    let matched = found.len();
    found.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    (found.into_iter().take(limit).map(|(_, e)| e).collect(), matched)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// A folder removed when the test ends.
    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("solidcraft-parts-library-{}-{n}", std::process::id()));
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

    fn library(files: &[&str]) -> (Dir, Library) {
        let dir = Dir::new();
        for f in files {
            let path = dir.path().join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
        }
        let library = scan(dir.path().to_str().unwrap()).unwrap();
        (dir, library)
    }

    #[test]
    fn a_folder_lists_one_file_per_part_with_its_folders_as_categories() {
        let (_dir, library) = library(&[
            "THRUSTERS/BLUE ROBOTICS/T200/T200.STEP",
            "THRUSTERS/BLUE ROBOTICS/T200/T200.IGS",
            "THRUSTERS/BLUE ROBOTICS/T200/T200.STL",
            "THRUSTERS/BLUE ROBOTICS/T100/T100-Thruster.IGS",
            "THRUSTERS/BLUE ROBOTICS/T100/T100-Thruster.SLDPRT",
            "THRUSTERS/BLUE ROBOTICS/CAD SOURCES.json",
            "THRUSTERS/.hidden/Secret.step",
            "Bracket.stp",
            "notes.pdf",
        ]);
        let listed: Vec<(&str, &str, &str)> = library.entries.iter().map(|e| (e.folder.as_str(), e.name.as_str(), e.format.as_str())).collect();
        assert_eq!(listed, [("", "Bracket", "stp"), ("THRUSTERS/BLUE ROBOTICS", "T100", "igs"), ("THRUSTERS/BLUE ROBOTICS", "T200", "step")]);
        assert!(library.entries.iter().all(|e| e.kind == Kind::Cad) && !library.truncated);
        assert!(library.absolute(&library.entries[2]).ends_with("THRUSTERS/BLUE ROBOTICS/T200/T200.STEP"));
    }

    #[test]
    fn a_design_is_listed_before_the_cad_file_it_was_made_from_with_its_properties() {
        let (dir, _) = library(&["THRUSTERS/T200.STEP"]);
        let mut doc = solidcraft_doc::Document::new("T200");
        doc.metadata.0.insert("part_number".into(), "BR-100545".into());
        doc.metadata.0.insert("description".into(), "T200 thruster".into());
        let bytes = crate::write_design_container(&doc).unwrap();
        std::fs::write(dir.path().join("THRUSTERS/T200.solidcraft"), bytes).unwrap();
        let library = scan(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(library.entries.len(), 1);
        let e = &library.entries[0];
        assert_eq!((e.kind, e.path.as_str()), (Kind::Design, "THRUSTERS/T200.solidcraft"));
        assert_eq!((e.part_number.as_deref(), e.description.as_deref(), e.revision.as_deref()), (Some("BR-100545"), Some("T200 thruster"), None));
        let (found, matched) = search(&library, "br-100545", 10);
        assert_eq!((found.len(), matched), (1, 1), "the part number is searched");
    }

    #[test]
    fn a_part_converted_as_sc_and_one_from_before_as_solidcraft_are_both_designs() {
        let (dir, _) = library(&["THRUSTERS/T200.STEP", "THRUSTERS/T500.STEP"]);
        let bytes = crate::write_design_container(&solidcraft_doc::Document::new("part")).unwrap();
        std::fs::write(dir.path().join("THRUSTERS/T200.sc"), &bytes).unwrap();
        std::fs::write(dir.path().join("THRUSTERS/T500.solidcraft"), &bytes).unwrap();
        let library = scan(dir.path().to_str().unwrap()).unwrap();
        let listed: Vec<(Kind, &str)> = library.entries.iter().map(|e| (e.kind, e.path.as_str())).collect();
        assert_eq!(listed, [(Kind::Design, "THRUSTERS/T200.sc"), (Kind::Design, "THRUSTERS/T500.solidcraft")]);
    }

    #[test]
    fn every_word_of_a_search_must_match_and_name_matches_come_first() {
        let (_dir, library) = library(&[
            "THRUSTERS/BLUE ROBOTICS/T200_THRUSTER_BR-100545/T200.STEP",
            "THRUSTERS/BLUE ROBOTICS/T500_THRUSTER_BR-101092/T500.STEP",
            "PENETRATORS/BLUE ROBOTICS/PENETRATOR-M-BLANK-10-25-R3.STEP",
            "PENETRATORS/BLUE ROBOTICS/Blank plug.STEP",
            "TOOLS/T200 THRUSTER WRENCH.step",
        ]);
        let names = |q: &str| search(&library, q, 10).0.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert_eq!(names("t200 thruster"), ["T200 THRUSTER BR-100545", "T200 THRUSTER WRENCH"]);
        assert_eq!(names("penetrator"), ["PENETRATOR-M-BLANK-10-25-R3", "Blank plug"], "a match in the name ranks above one in the folders");
        assert_eq!(names("blue robotics"), ["Blank plug", "PENETRATOR-M-BLANK-10-25-R3", "T200 THRUSTER BR-100545", "T500 THRUSTER BR-101092"]);
        assert!(names("thruster camera").is_empty(), "every word must match");
        assert_eq!(search(&library, "  ", 2), (library.entries.iter().take(2).collect(), 5), "a blank search lists every part");
    }

    #[test]
    fn a_part_alone_in_a_folder_named_after_it_takes_the_folders_name() {
        let (_dir, library) = library(&[
            "FITTINGS/BLUE ROBOTICS/M10_TO_M14_BULKHEAD_ADAPTER_BR-102909/M10_TO_M14_BULKHEAD_ADAPTER_BR-102909/BR-102512_RevA.STEP",
            "FITTINGS/BLUE ROBOTICS/M06_TO_M10_BULKHEAD_ADAPTER_BR-102851/BR-102511_RevA.STEP",
            "FITTINGS/BLUE ROBOTICS/M06_TO_M10_BULKHEAD_ADAPTER_BR-102851/WL-M10-NUT-PUBLIC.STEP",
            "CAMERAS/Low-light camera.igs",
        ]);
        let listed: Vec<(&str, &str)> = library.entries.iter().map(|e| (e.folder.as_str(), e.name.as_str())).collect();
        assert_eq!(
            listed,
            [
                ("CAMERAS", "Low-light camera"),
                ("FITTINGS/BLUE ROBOTICS", "M10 TO M14 BULKHEAD ADAPTER BR-102909"),
                ("FITTINGS/BLUE ROBOTICS/M06_TO_M10_BULKHEAD_ADAPTER_BR-102851", "BR-102511_RevA"),
                ("FITTINGS/BLUE ROBOTICS/M06_TO_M10_BULKHEAD_ADAPTER_BR-102851", "WL-M10-NUT-PUBLIC"),
            ],
            "a folder of several parts and a category folder keep the files' names"
        );
        let (found, _) = search(&library, "br-102512", 5);
        assert_eq!(found.len(), 1, "the file's own name is still searched");
    }

    #[test]
    fn a_vendor_part_number_is_found_in_the_name_or_the_file_name() {
        let entry = |name: &str, path: &str| Entry {
            path: path.into(),
            name: name.into(),
            folder: String::new(),
            kind: Kind::Cad,
            format: "step".into(),
            bytes: 0,
            part_number: None,
            description: None,
            revision: None,
            local: true,
        };
        let guess = |name: &str, path: &str| guess_part_number(&entry(name, path));
        assert_eq!(guess("M10 TO M14 BULKHEAD ADAPTER BR-102909", "A/BR-102512_RevA.STEP").as_deref(), Some("BR-102909"), "the name first");
        assert_eq!(guess("Adapter", "A/BR-102512_RevA.STEP").as_deref(), Some("BR-102512"));
        assert_eq!(guess("PUB-BR-101303-004_RevC", "x.step").as_deref(), Some("BR-101303"), "the letters just before the digits");
        assert_eq!(guess("wl-m14-nut", "wl-m14-nut.step"), None, "two digits are not a part number");
        assert_eq!(guess("T200", "T200.STEP"), None);
        assert_eq!(guess("ABCDEF-12345", "x.step"), None, "more than five letters");
    }

    #[test]
    fn a_missing_or_unreadable_folder_is_refused() {
        let dir = Dir::new();
        assert!(scan(dir.path().join("missing").to_str().unwrap()).is_err());
        let file = dir.path().join("part.step");
        std::fs::write(&file, b"x").unwrap();
        assert!(scan(file.to_str().unwrap()).is_err(), "a file is not a library folder");
        assert!(scan("").is_err());
    }
}
