#![allow(clippy::unwrap_used)]
use serde_json::json;

use crate::scenario::Harness;

fn settle(h: &mut Harness, done: impl Fn(&crate::SolveApp) -> bool) {
    for _ in 0..600 {
        h.app.versions.soon();
        h.frames(1);
        crate::save_job::tick(&mut h.app, false);
        if done(&h.app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("never settled");
}

fn status(app: &crate::SolveApp) -> Option<serde_json::Value> {
    app.versions.status.as_ref().and_then(|s| s.as_ref().ok()).cloned()
}

fn history_len(app: &crate::SolveApp) -> usize {
    app.versions.history.as_ref().and_then(|h| h.as_ref().ok()).and_then(|h| h["commits"].as_array().map(Vec::len)).unwrap_or(0)
}

#[test]
fn a_part_on_its_own_keeps_versions_checks_in_and_is_released() {
    let mut h = Harness::new();
    let folder = h.dir.join("JIGS");
    std::fs::create_dir_all(&folder).unwrap();
    let part = folder.join("Tether clip.solidcraft");
    h.app.run("solid.box", json!({"length": 10, "width": 20, "height": 3, "body_name": "Clip"})).unwrap();
    h.app.run("file.save", json!({"path": part.to_string_lossy()})).unwrap();
    h.app.preferences.versions_tab = true;
    h.frames(2);
    // The header has a Versions tab; a part outside every kept folder offers to keep its folder.
    h.step(&json!({"widget": "Show Versions"})).unwrap();
    assert!(h.app.versions.open);
    h.frames(2);
    h.step(&json!({"widget": "Keep Versions of “JIGS”"})).unwrap();
    settle(&mut h, |app| status(app).is_some_and(|s| s["changes"].as_array().is_some_and(|c| c.len() == 1)));
    assert_eq!(h.app.preferences.versions_folders.len(), 1);
    assert!(h.app.preferences.versions_folders[0].ends_with("JIGS"));
    // Check in with a revision: the part's Revision property is set and saved first.
    h.app.versions.message = "First clip".into();
    h.app.versions.release = true;
    h.frames(2);
    h.step(&json!({"widget": "Check In 1 Part"})).unwrap();
    settle(&mut h, |app| history_len(app) == 1 && status(app).is_some_and(|s| s["changes"].as_array().is_some_and(Vec::is_empty)));
    let history = h.app.versions.history.clone().unwrap().unwrap();
    assert_eq!(history["commits"][0]["message"], "First clip");
    assert_eq!(history["commits"][0]["revisions"], json!([{"path": "Tether clip.solidcraft", "revision": "Rev A"}]));
    let props = h.app.run("document.metadata", json!({})).unwrap_or_default();
    assert!(props.to_string().contains("Rev A"), "{props}");

    // A change shows in My changes, and restoring the first version puts it back.
    h.app.run("solid.box", json!({"length": 5, "width": 5, "height": 5, "body_name": "Tab"})).unwrap();
    h.app.run("file.save", json!({"path": part.to_string_lossy()})).unwrap();
    settle(&mut h, |app| status(app).is_some_and(|s| s["changes"].as_array().is_some_and(|c| c.len() == 1)));
    let first = history["commits"][0]["id"].as_str().unwrap().to_string();
    h.app.run("versions.restore", json!({"folder": folder.to_string_lossy(), "commit": first, "path": "Tether clip.solidcraft"})).unwrap();
    settle(&mut h, |app| status(app).is_some_and(|s| s["changes"].as_array().is_some_and(Vec::is_empty)));
}
