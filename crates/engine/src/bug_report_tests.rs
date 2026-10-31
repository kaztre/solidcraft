#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;

use super::*;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("solidcraft-bug-report-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

fn boxed() -> Session {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 40, "width": 20, "height": 3})).unwrap();
    s
}

fn request(title: &str) -> Request {
    Request { title: title.into(), what_happened: "it stopped".into(), steps: "1. box\n2. thread".into(), ..Request::default() }
}

#[test]
fn a_report_holds_what_was_written_the_design_and_every_feature() {
    let d = dir("holds");
    let s = boxed();
    let mut r = request("Thread: modelled thread is not made");
    r.dialog = Some(json!([{"command": "solid.thread", "params": {"modelled": true}}]));
    r.context = "selection: face 3".into();
    let v = write(&s, &d, &r).unwrap();
    assert_eq!(v["format"], FORMAT);
    assert_eq!(v["title"], "Thread: modelled thread is not made");
    assert_eq!(v["steps"], "1. box\n2. thread");
    assert_eq!(v["status"]["state"], "open");
    assert_eq!(v["dialog_commands"][0]["command"], "solid.thread");
    assert_eq!(v["context"], "selection: face 3");
    assert_eq!(v["features"].as_array().unwrap().len(), s.doc.features.len());
    assert!(v["features"][0]["error"].is_null());
    // The design opens again as it was.
    let design = v["design_path"].as_str().unwrap();
    let back = solidcraft_io::read_design(&fs::read(design).unwrap()).unwrap();
    assert_eq!(back.features.len(), s.doc.features.len());
    // Tests install no crash journal or log: the report says what it lacks instead of failing.
    let notes = v["notes"].to_string();
    assert!(notes.contains("no command journal"), "{notes}");
    assert!(notes.contains("no log"), "{notes}");
    assert!(v["reproduce"].is_null());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn reports_in_the_same_second_get_their_own_folders_and_list_newest_first() {
    let d = dir("same-second");
    let s = boxed();
    let a = write(&s, &d, &request("first")).unwrap();
    let b = write(&s, &d, &request("second")).unwrap();
    assert_ne!(a["id"], b["id"]);
    let listed = list(&d);
    assert_eq!(listed.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["second", "first"]);
    assert!(listed.iter().all(|r| r.status == "open" && r.issue.is_none()));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn status_records_the_issue_and_the_fixing_commit_and_clear_takes_only_fixed_ones() {
    let d = dir("status");
    let s = boxed();
    let open = write(&s, &d, &request("still open")).unwrap();
    let done = write(&s, &d, &request("fixed one")).unwrap();
    let id = done["id"].as_str().unwrap();
    set_status(&d, id, None, Some(12), None).unwrap();
    let v = set_status(&d, id, Some("fixed"), None, Some("31535b79")).unwrap();
    assert_eq!(v["status"]["state"], "fixed");
    assert_eq!(v["status"]["issue"], 12);
    assert_eq!(v["status"]["commit"], "31535b79");
    // Paths are added when read, never saved into the report.
    let saved: Value = serde_json::from_slice(&fs::read(d.join(id).join("report.json")).unwrap()).unwrap();
    assert!(saved.get("path").is_none() && saved.get("design_path").is_none());
    assert!(set_status(&d, id, Some("done"), None, None).unwrap_err().contains("open or fixed"));
    assert!(set_status(&d, id, None, None, Some("main; rm -rf")).unwrap_err().contains("git hash"));
    assert_eq!(clear(&d, Clear::Fixed), 1);
    assert_eq!(list(&d).iter().map(|r| r.id.clone()).collect::<Vec<_>>(), [open["id"].as_str().unwrap()]);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn an_id_that_is_not_a_report_folder_reads_nothing() {
    let d = dir("ids");
    fs::create_dir_all(&d).unwrap();
    for id in ["", "../crashes", "20261031-120000-001/../x", "abc"] {
        assert!(read(&d, id).is_err(), "{id}");
        assert!(set_status(&d, id, Some("fixed"), None, None).is_err(), "{id}");
    }
    assert_eq!(clear(&d, Clear::One("..")), 0);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn the_log_copy_keeps_whole_lines_from_the_end() {
    let d = dir("tail");
    fs::create_dir_all(&d).unwrap();
    let path = d.join("run.log");
    let text: String = (0..100).map(|i| format!("line {i:03}\n")).collect();
    fs::write(&path, &text).unwrap();
    let (b, cut) = tail(&path, 30).unwrap();
    let got = String::from_utf8(b).unwrap();
    assert!(cut);
    assert!(got.starts_with("line ") && got.ends_with("line 099\n"), "{got:?}");
    let (all, cut) = tail(&path, 10_000).unwrap();
    assert!(!cut);
    assert_eq!(all, text.as_bytes());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn the_command_saves_a_report_with_its_screenshot_and_refuses_bad_input() {
    let mut s = boxed();
    let undo_before = s.undo.len();
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), b"not really an image"].concat();
    let reply = s
        .execute("diagnostics.bug_report", &json!({"title": "Fillet: wrong radius", "screenshot_png": solidcraft_doc::canvas::base64_encode(&png)}))
        .unwrap();
    let files: Vec<&str> = reply["files"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(files.contains(&"design") && files.contains(&"screenshot"), "{files:?}");
    let id = reply["id"].as_str().unwrap();
    let back = read(&default_dir().unwrap(), id).unwrap();
    assert_eq!(fs::read(back["screenshot_path"].as_str().unwrap()).unwrap(), png);
    for (p, why) in [
        (json!({}), "title"),
        (json!({"title": "  "}), "title"),
        (json!({"title": "x", "screenshot_png": "aGVsbG8="}), "PNG"),
        (json!({"title": "x", "what_happened": 4}), "text"),
    ] {
        let e = s.execute("diagnostics.bug_report", &p).unwrap_err().to_string();
        assert!(e.contains(why), "{p}: {e}");
    }
    // Saving a report changes nothing in the design and leaves no undo step.
    assert_eq!(s.undo.len(), undo_before);
}
