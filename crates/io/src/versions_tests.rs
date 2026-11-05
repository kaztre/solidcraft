use super::*;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("solidcraft-versions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join("work").join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.0.join("work").join(rel)).unwrap()
    }
    fn store(&self) -> Store {
        std::fs::create_dir_all(self.0.join("work")).unwrap();
        Store::locate(&self.0.join("work"), &self.0.join("history"))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn me() -> Author {
    Author::new("Sam", "sam@example.com")
}

fn changes(s: &Store) -> Vec<(String, Change)> {
    s.status().unwrap().changes.into_iter().map(|c| (c.path, c.change)).collect()
}

/// Bumps a file's date, as a save a moment later would (some file systems keep whole seconds).
fn touch_later(path: &Path) {
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
}

#[test]
fn a_part_is_checked_in_changed_and_put_back_to_its_first_version() {
    let d = Dir::new("round-trip");
    d.write("PROJECTS/ROV/Frame plate.solidcraft", "plate 5 mm");
    d.write("PROJECTS/ROV/Thruster mount.solidcraft", "mount 18 mm");
    d.write("VENDOR/T200.STEP", "ISO-10303-21; t200");
    d.write("VENDOR/T200.stl", "solid mesh");
    d.write("VENDOR/datasheet.pdf", "pdf");
    // Other programs' scratch, never kept.
    d.write("VENDOR/~$datasheet.docx", "word's lock");
    d.write("VENDOR/T200.STEP.bak", "backup");
    d.write("ELECTRICAL/pod.kicad_prl", "window settings");
    d.write("ELECTRICAL/_autosave-pod.kicad_sch", "autosave");
    d.write("ELECTRICAL/pod-backups/pod-2026.zip", "backup");
    d.write("ELECTRICAL/fp-info-cache", "cache");
    d.write("node_modules/x/a.js", "build");
    let s = d.store();
    assert!(!s.exists());
    s.start().unwrap();
    assert!(s.exists());
    // Every file is kept, the scratch left out.
    assert_eq!(
        changes(&s),
        vec![
            ("PROJECTS/ROV/Frame plate.solidcraft".into(), Change::Added),
            ("PROJECTS/ROV/Thruster mount.solidcraft".into(), Change::Added),
            ("VENDOR/T200.STEP".into(), Change::Added),
            ("VENDOR/T200.stl".into(), Change::Added),
            ("VENDOR/datasheet.pdf".into(), Change::Added),
        ]
    );
    let all: Vec<String> = s.status().unwrap().changes.into_iter().map(|c| c.path).collect();
    let first = s.check_in(&all, "First parts", &me(), 1_700_000_000).unwrap();
    assert!(changes(&s).is_empty());
    assert_eq!(first.author.name, "Sam");
    assert_eq!(first.time, 1_700_000_000);

    d.write("PROJECTS/ROV/Frame plate.solidcraft", "plate 6 mm");
    touch_later(&d.0.join("work/PROJECTS/ROV/Frame plate.solidcraft"));
    assert_eq!(changes(&s), vec![("PROJECTS/ROV/Frame plate.solidcraft".into(), Change::Changed)]);
    let second = s.check_in(&["PROJECTS/ROV/Frame plate.solidcraft".into()], "Thicker plate", &me(), 1_700_000_100).unwrap();
    assert_eq!(second.parents, vec![first.id.clone()]);
    let e = s.check_in(&["PROJECTS/ROV/Frame plate.solidcraft".into()], "Again", &me(), 1_700_000_200).unwrap_err();
    assert!(e.to_string().contains("nothing changed"), "{e}");

    let (log, more) = s.log(None, 10).unwrap();
    assert!(!more);
    assert_eq!(log.iter().map(|e| e.commit.message.as_str()).collect::<Vec<_>>(), ["Thicker plate", "First parts"]);
    assert_eq!(log[0].files, vec![("PROJECTS/ROV/Frame plate.solidcraft".to_string(), Change::Changed)]);
    assert_eq!(log[1].files.len(), 5);
    let (mount, _) = s.log(Some("PROJECTS/ROV/Thruster mount.solidcraft"), 10).unwrap();
    assert_eq!(mount.len(), 1, "the mount only changed in the first version");
    let (one, more) = s.log(None, 1).unwrap();
    assert!(more && one.len() == 1);

    // Restoring one part writes it back; the folder then shows it as a change.
    assert_eq!(s.restore(&first.id, Some("PROJECTS/ROV/Frame plate.solidcraft")).unwrap(), ["PROJECTS/ROV/Frame plate.solidcraft"]);
    assert_eq!(d.read("PROJECTS/ROV/Frame plate.solidcraft"), "plate 5 mm");
    assert_eq!(changes(&s), vec![("PROJECTS/ROV/Frame plate.solidcraft".into(), Change::Changed)]);
    // Restoring the whole folder leaves files that already match alone.
    assert_eq!(s.restore(&second.id, None).unwrap(), ["PROJECTS/ROV/Frame plate.solidcraft"]);
    assert_eq!(d.read("PROJECTS/ROV/Frame plate.solidcraft"), "plate 6 mm");
    assert!(changes(&s).is_empty());

    // A part removed from the folder is checked in as removed.
    std::fs::remove_file(d.0.join("work/VENDOR/T200.STEP")).unwrap();
    assert_eq!(changes(&s), vec![("VENDOR/T200.STEP".into(), Change::Deleted)]);
    s.check_in(&["VENDOR/T200.STEP".into()], "Drop the vendor file", &me(), 1_700_000_300).unwrap();
    assert!(changes(&s).is_empty());
    assert!(s.file_in(&second.id, "VENDOR/T200.STEP").unwrap().is_some());
}

#[test]
fn names_revisions_and_copies_stay_with_their_versions() {
    let d = Dir::new("names");
    d.write("Bracket.solidcraft", "bracket a");
    let s = d.store();
    s.start().unwrap();
    let v1 = s.check_in(&["Bracket.solidcraft".into()], "Bracket", &me(), 1_700_000_000).unwrap();
    let rev = s.name(&v1.id, None, Some(("Bracket.solidcraft", "Rev A")), &me(), 1_700_000_001).unwrap();
    assert_eq!(rev.revision.as_deref(), Some("Rev A"));
    let e = s.name(&v1.id, None, Some(("Bracket.solidcraft", "Rev A")), &me(), 1_700_000_002).unwrap_err();
    assert!(e.to_string().contains("already has"), "{e}");
    let version = s.name(&v1.id, Some("Sea trial 2"), None, &me(), 1_700_000_003).unwrap();
    assert_eq!(version.name, "Sea-trial-2");
    assert!(s.name(&v1.id, Some("Sea trial 2"), None, &me(), 1_700_000_004).is_err());
    assert!(s.name(&v1.id, None, Some(("Missing.solidcraft", "Rev A")), &me(), 1_700_000_005).is_err());
    let tags = s.tags().unwrap();
    assert_eq!(tags.len(), 2);
    assert!(tags.iter().all(|t| t.commit == v1.id));

    // Save a version as a new part: the next check-in of the copy says where it came from.
    let dest = d.0.join("work/Bracket (copy).solidcraft");
    s.copy_out(&v1.id, "Bracket.solidcraft", &dest, Some(("Bracket (copy).solidcraft", "Copied from Bracket.solidcraft (v1)"))).unwrap();
    assert_eq!(d.read("Bracket (copy).solidcraft"), "bracket a");
    assert!(s.copy_out(&v1.id, "Bracket.solidcraft", &dest, None).is_err(), "never overwrites");
    let c = s.check_in(&["Bracket (copy).solidcraft".into()], "Longer bracket", &me(), 1_700_000_010).unwrap();
    assert_eq!(c.message, "Longer bracket\n\nCopied from Bracket.solidcraft (v1)");
}

#[test]
fn hostile_paths_ids_and_damaged_history_are_refused_without_panicking() {
    let d = Dir::new("hostile");
    d.write("Part.solidcraft", "part");
    let s = d.store();
    s.start().unwrap();
    let v = s.check_in(&["Part.solidcraft".into()], "Part", &me(), 1).unwrap();
    for bad in ["", "../Part.solidcraft", "/etc/passwd", ".git/config", "a//b.solidcraft", "a\\b.solidcraft", "x/./y.step"] {
        assert!(check_path(bad).is_err(), "{bad:?}");
        assert!(s.check_in(&[bad.to_string()], "x", &me(), 2).is_err(), "{bad:?}");
        assert!(s.restore(&v.id, Some(bad)).is_err(), "{bad:?}");
    }
    assert!(s.check_in(&["Part.solidcraft".into()], "   ", &me(), 2).is_err(), "a message is required");
    d.write("Part.solidcraft.bak", "x");
    assert!(s.check_in(&["Part.solidcraft.bak".into()], "x", &me(), 2).is_err(), "never a backup or lock file");
    for id in ["", "zz", "../../../../etc/passwd", &"A".repeat(40), &"0".repeat(40)] {
        assert!(s.read_commit(id).is_err(), "{id:?}");
        assert!(s.restore(id, None).is_err(), "{id:?}");
    }
    // Garbage where an object should be.
    let path = s.object_path(&v.id).unwrap();
    std::fs::write(&path, b"not zlib at all").unwrap();
    assert!(s.read_commit(&v.id).is_err());
    assert!(s.log(None, 10).is_err());
    std::fs::write(&path, b"").unwrap();
    assert!(s.read_commit(&v.id).is_err());
    let names = Author::new("<evil>\nname", "a>b@c");
    assert!(!names.name.contains(['<', '>', '\n']) && !names.email.contains('>'));
}

/// The history is ordinary git: when git is installed, it reads it back and finds no fault.
#[test]
fn git_itself_reads_the_history() {
    let Ok(out) = std::process::Command::new("git").arg("--version").output() else { return };
    if !out.status.success() {
        return;
    }
    let d = Dir::new("git");
    d.write("A/Plate.solidcraft", "plate");
    d.write("A/B/Mount.step", "mount");
    d.write("Z.stp", "z");
    let s = d.store();
    s.start().unwrap();
    let v = s.check_in(&["A/Plate.solidcraft".into(), "A/B/Mount.step".into(), "Z.stp".into()], "First", &me(), 1_700_000_000).unwrap();
    d.write("A/Plate.solidcraft", "plate 2");
    s.check_in(&["A/Plate.solidcraft".into()], "Second", &me(), 1_700_000_100).unwrap();
    s.name(&v.id, Some("v1.0"), None, &me(), 1_700_000_200).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").arg("--git-dir").arg(&s.git).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    git(&["fsck", "--strict", "--no-dangling"]);
    assert_eq!(git(&["log", "--format=%s|%an", "main"]).lines().collect::<Vec<_>>(), ["Second|Sam", "First|Sam"]);
    assert_eq!(git(&["show", "main:A/Plate.solidcraft"]), "plate 2");
    assert_eq!(git(&["show", "v1.0:A/Plate.solidcraft"]), "plate");
    assert_eq!(git(&["ls-tree", "-r", "--name-only", "main"]).lines().collect::<Vec<_>>(), ["A/B/Mount.step", "A/Plate.solidcraft", "Z.stp"]);
}
