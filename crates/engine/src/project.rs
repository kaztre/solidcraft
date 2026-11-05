//! The project folder the assistant reads (docs/agent-chat.md, "The project folder"): the folder
//! that keeps versions of the design in front, else the nearest folder above it with a
//! `PROJECT.md` or its own git history, else the folder the design is in. The desktop app
//! says which it is ([`set_folder`]) and whether the user lets the assistant read it
//! ([`set_allowed`], Preferences → AI); without the app, it is the folder of the session's design.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

static FOLDER: RwLock<Option<PathBuf>> = RwLock::new(None);
static ALLOWED: AtomicBool = AtomicBool::new(true);

/// The project folder of the design in front, as the app works it out; `None` leaves it to the
/// session's design.
pub fn set_folder(folder: Option<PathBuf>) {
    if FOLDER.read().is_ok_and(|f| *f == folder) {
        return;
    }
    if let Ok(mut f) = FOLDER.write() {
        *f = folder;
    }
}

/// Whether the user lets the assistant read the project folder (on unless turned off).
pub fn set_allowed(allowed: bool) {
    ALLOWED.store(allowed, Ordering::Relaxed);
}

pub fn allowed() -> bool {
    ALLOWED.load(Ordering::Relaxed)
}

/// The project folder: the app's choice, else the folder `design` (a saved design's path) is in.
pub fn folder(design: Option<&str>) -> Option<PathBuf> {
    FOLDER.read().ok().and_then(|f| f.clone()).or_else(|| folder_of(design?, &[]))
}

/// Folders above a design looked at for a project's top, at most.
const MAX_UP: usize = 8;

/// The project folder of the design at `path`: the innermost of `versioned` (folders that keep
/// versions) holding it, else the nearest folder holding it that has a `PROJECT.md` or a `.git`
/// (never the home folder or above), else the folder it is in.
pub fn folder_of(path: &str, versioned: &[String]) -> Option<PathBuf> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return None;
    }
    if let Some(f) = versioned.iter().filter(|f| p.starts_with(f.as_str())).max_by_key(|f| f.len()) {
        return Some(PathBuf::from(f));
    }
    let own = p.parent().filter(|d| !d.as_os_str().is_empty())?;
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let top = own
        .ancestors()
        .take(MAX_UP)
        .take_while(|d| d.parent().is_some() && home.as_deref() != Some(*d))
        .find(|d| d.join("PROJECT.md").is_file() || d.join(".git").exists());
    Some(top.unwrap_or(own).to_path_buf())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_design_is_in_its_versions_folder_else_its_own_folder() {
        let versioned = vec!["/work/Pod".to_string(), "/work/Pod/Electrical".to_string(), "/work/Other".to_string()];
        assert_eq!(folder_of("/work/Pod/CAD/a.sc", &versioned), Some(PathBuf::from("/work/Pod")));
        assert_eq!(folder_of("/work/Pod/Electrical/b.sc", &versioned), Some(PathBuf::from("/work/Pod/Electrical")), "the innermost");
        assert_eq!(folder_of("/work/Podium/a.sc", &versioned), Some(PathBuf::from("/work/Podium")), "a folder, not a prefix of the name");
        assert_eq!(folder_of("a.sc", &versioned), None, "an unsaved or relative design has none");
    }

    #[test]
    fn a_design_in_a_sub_folder_finds_its_projects_top_by_its_brief() {
        let dir = std::env::temp_dir().join(format!("solidcraft-project-top-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Pod/CAD/Frame")).unwrap();
        std::fs::create_dir_all(dir.join("Loose")).unwrap();
        std::fs::write(dir.join("Pod/PROJECT.md"), "# Pod").unwrap();
        let design = dir.join("Pod/CAD/Frame/plate.sc");
        assert_eq!(folder_of(design.to_str().unwrap(), &[]), Some(dir.join("Pod")));
        let loose = dir.join("Loose/a.sc");
        assert_eq!(folder_of(loose.to_str().unwrap(), &[]), Some(dir.join("Loose")), "no brief above: its own folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
