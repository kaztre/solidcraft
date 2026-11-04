//! CAD files that come off the internet (`file.download`): what a downloaded file is, the CAD
//! files in a vendor's zip, and a name it is safe to save each under. Every byte here is a
//! stranger's: names are cut to their last part and cleaned, and a zip is read within limits.

use std::io::Read;

/// Is this a CAD file SolidCraft reads: STEP, IGES or another CAD format ([`crate::is_step_path`]),
/// or a mesh ([`crate::is_mesh_path`])?
pub fn is_cad_path(path: &str) -> bool {
    crate::is_step_path(path) || crate::is_mesh_path(path)
}

/// `name` made safe to save in a folder of the user's: its last part only (no folders, so a
/// zip's `../../x.step` stays where it is put), without control characters or the ones a file
/// system refuses, without leading dots or spaces, at most 120 characters, and a CAD file's or a
/// zip's. `None` otherwise.
pub fn safe_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next()?;
    let cleaned: String =
        last.chars().filter(|c| !c.is_control()).map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']);
    let mut out: String = trimmed.chars().take(120).collect();
    // A cut name keeps its extension.
    if trimmed.chars().count() > 120
        && let Some((_, ext)) = trimmed.rsplit_once('.')
        && ext.len() <= 8
    {
        out = format!("{}.{ext}", out.chars().take(119 - ext.len()).collect::<String>());
    }
    (!out.is_empty() && (is_cad_path(&out) || is_zip_name(&out))).then_some(out)
}

/// Is `name` a zip archive's?
pub fn is_zip_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("zip"))
}

/// The extension a file with these first bytes has, when its name gives none SolidCraft reads:
/// `step` (ISO 10303-21's header), `zip`, `stl` (an ASCII STL's first word) or `glb`.
pub fn sniffed_extension(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with("ISO-10303-21") {
        Some("step")
    } else if head.starts_with(b"PK\x03\x04") {
        Some("zip")
    } else if head.starts_with(b"glTF") {
        Some("glb")
    } else if text.starts_with("solid ") && text.contains("facet") {
        Some("stl")
    } else {
        None
    }
}

/// The CAD files in the zip `bytes`, each with its [`safe_name`], in the archive's order: at most
/// `max_files` of them and `max_bytes` expanded in all (a file past that is refused, not cut).
/// Folders, macOS's `__MACOSX` copies, hidden files and anything that isn't a CAD file are left
/// out.
pub fn cad_files_in_zip(bytes: &[u8], max_files: usize, max_bytes: usize) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a zip file SolidCraft can read: {e}"))?;
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut left = max_bytes;
    for i in 0..archive.len() {
        if out.len() >= max_files {
            break;
        }
        let mut entry = archive.by_index(i).map_err(|e| format!("the zip file is damaged: {e}"))?;
        let full = entry.name().map_err(|e| format!("the zip file is damaged: {e}"))?.to_string();
        if entry.is_dir() || full.split(['/', '\\']).any(|p| p == "__MACOSX" || p.starts_with('.')) {
            continue;
        }
        let Some(name) = safe_name(&full).filter(|n| is_cad_path(n)) else { continue };
        if entry.size() > left as u64 {
            return Err(format!("{name} in the zip file is larger than SolidCraft reads"));
        }
        let mut data = Vec::new();
        entry.by_ref().take(left as u64 + 1).read_to_end(&mut data).map_err(|e| format!("{name} in the zip file can't be read: {e}"))?;
        if data.len() > left {
            return Err(format!("{name} in the zip file is larger than SolidCraft reads"));
        }
        left -= data.len();
        out.push((name, data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut out);
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn a_name_keeps_its_last_part_cleaned_and_must_be_cad() {
        assert_eq!(safe_name("../../etc/T200.STEP").as_deref(), Some("T200.STEP"));
        assert_eq!(safe_name("C:\\parts\\bracket:v2.stp").as_deref(), Some("bracket_v2.stp"));
        assert_eq!(safe_name("..hidden.step").as_deref(), Some("hidden.step"));
        assert_eq!(safe_name("model.zip").as_deref(), Some("model.zip"));
        for bad in ["", "notes.pdf", "setup.exe", "...", "/", "readme"] {
            assert_eq!(safe_name(bad), None, "{bad:?}");
        }
        let long = format!("{}.step", "a".repeat(300));
        let cut = safe_name(&long).unwrap();
        assert!(cut.chars().count() <= 120 && cut.ends_with(".step"), "{cut}");
    }

    #[test]
    fn a_file_with_no_known_name_is_told_by_its_first_bytes() {
        assert_eq!(sniffed_extension(b"ISO-10303-21;\nHEADER;"), Some("step"));
        assert_eq!(sniffed_extension(b"\xef\xbb\xbfISO-10303-21;"), Some("step"));
        assert_eq!(sniffed_extension(&zip_of(&[("a.step", b"x")])), Some("zip"));
        assert_eq!(sniffed_extension(b"solid part\n facet normal 0 0 1"), Some("stl"));
        assert_eq!(sniffed_extension(b"<html><body>Sign in</body></html>"), None);
    }

    #[test]
    fn a_zip_gives_its_cad_files_only_and_within_its_limits() {
        let z = zip_of(&[
            ("vendor/T200/T200.STEP", b"ISO-10303-21;"),
            ("vendor/T200/readme.pdf", b"%PDF"),
            ("__MACOSX/vendor/T200/._T200.STEP", b"junk"),
            ("vendor/T200/.hidden.step", b"x"),
            ("../escape.stl", b"solid a"),
            ("vendor/T200/mesh/T200.stl", b"solid a"),
        ]);
        let files = cad_files_in_zip(&z, 10, 1 << 20).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["T200.STEP", "T200.stl"], "a path through `..` is skipped, a mesh in a folder kept");
        assert_eq!(cad_files_in_zip(&z, 1, 1 << 20).unwrap().len(), 1, "at most max_files");
        assert!(cad_files_in_zip(&z, 10, 5).is_err(), "past max_bytes is refused");
        assert!(cad_files_in_zip(b"not a zip", 10, 100).is_err());
    }
}
