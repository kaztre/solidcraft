//! Bug reports the user writes (Report a bug): everything needed to find the fault, kept on this
//! computer in `reports/` beside the crash folder (docs/crash-reports.md). Unlike crash reports
//! they are never pruned: each stays until it is deleted, marked `open` or `fixed`.
//!
//! A report is a folder `reports/<time>-<n>/`:
//! - `report.json`: what the user wrote, the app and system, the command the open dialog would
//!   run, every feature's error and warning, this run's recent commands, notes, and its status
//!   (`open` or `fixed`, with the issue and commit that track it);
//! - `design.solidcraft`: the design as it is now, unsaved changes included;
//! - `start.solidcraft` and `replay.json`: the design this run's commands start from and those
//!   commands, when the crash journal has them (`solidcraft-cli reports replay <id>`);
//! - `log.txt`: this run's log ([`crate::session_log`]), at most its last [`MAX_LOG_BYTES`];
//! - `screenshot.png`, when the report window took one.

use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{Session, crash, session_log};

pub const FORMAT: &str = "solidcraft-report/1";
/// Reports kept at most: a new one past this is refused until some are deleted.
pub const MAX_REPORTS: usize = 500;
/// The design copied at most.
pub const MAX_DESIGN_BYTES: usize = 64 << 20;
/// The end of the log copied at most.
pub const MAX_LOG_BYTES: u64 = 2 << 20;
pub const MAX_SCREENSHOT_BYTES: usize = 16 << 20;
/// One text field (title, answers, context) at most.
pub const MAX_TEXT: usize = 20_000;
/// The dialog's commands at most, as JSON.
const MAX_DIALOG_BYTES: usize = 200_000;
const STATUSES: [&str; 2] = ["open", "fixed"];

/// What the report window hands over.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub title: String,
    pub what_happened: String,
    pub expected: String,
    pub steps: String,
    /// The commands the open dialog would run, when it was open.
    pub dialog: Option<Value>,
    /// What the front end knows and the engine does not: selection, preferences, window, GPU.
    pub context: String,
    pub screenshot_png: Option<Vec<u8>>,
}

/// One report, as listed.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Summary {
    pub id: String,
    pub time: String,
    pub title: String,
    pub status: String,
    pub issue: Option<u64>,
    pub commit: Option<String>,
    pub version: String,
    pub path: String,
    pub replayable: bool,
}

/// The folder reports go in: `reports/` beside the crash and recovery folders.
pub fn default_dir() -> Option<PathBuf> {
    // Tests never touch the user's real reports.
    if cfg!(test) {
        return Some(std::env::temp_dir().join(format!("solidcraft-test-reports-{}", std::process::id())));
    }
    if let Some(d) = solidcraft_geom::env::var_os("REPORT_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    crate::recovery::default_dir().and_then(|d| d.parent().map(|p| p.join("reports")))
}

fn limited(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} […]", &s[..end])
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn ids(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().join("report.json").is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| valid_id(n))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// A new, empty report folder: `<time>-<n>`, claimed with `create_dir` so two reports in the same
/// second never share one.
fn claim(dir: &Path, secs: u64) -> std::result::Result<(String, PathBuf), String> {
    let stamp = crash::stamp(secs);
    for n in 1..1000 {
        let id = format!("{stamp}-{n:03}");
        let folder = dir.join(&id);
        match fs::create_dir(&folder) {
            Ok(()) => return Ok((id, folder)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", folder.display())),
        }
    }
    Err("too many reports in one second".into())
}

/// The last `max` bytes of `path`, from a line start when it had to cut.
fn tail(path: &Path, max: u64) -> Option<(Vec<u8>, bool)> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let cut = len > max;
    if cut {
        f.seek(SeekFrom::Start(len - max)).ok()?;
    }
    let mut b = Vec::new();
    f.take(max).read_to_end(&mut b).ok()?;
    if cut && let Some(nl) = b.iter().position(|&c| c == b'\n') {
        b.drain(..=nl);
    }
    Some((b, cut))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Writes one of a report's files, recording it under `key`, or a note when it could not be written.
fn put(folder: &Path, files: &mut serde_json::Map<String, Value>, notes: &mut Vec<String>, key: &str, name: &str, bytes: &[u8]) {
    match solidcraft_io::write_atomic(&folder.join(name), bytes, false) {
        Ok(()) => {
            files.insert(key.into(), json!(name));
        }
        Err(e) => notes.push(format!("{name} could not be written: {e}")),
    }
}

/// Writes a report of the session as it is now into `dir`. Returns its `report.json` with the
/// paths of its files (as [`read`] does).
pub fn write(s: &Session, dir: &Path, r: &Request) -> std::result::Result<Value, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if ids(dir).len() >= MAX_REPORTS {
        return Err(format!("{MAX_REPORTS} reports are kept already: delete fixed ones (`solidcraft-cli reports clear --fixed`)"));
    }
    let t = now();
    let (id, folder) = claim(dir, t)?;
    let mut files = serde_json::Map::new();
    let mut notes: Vec<String> = Vec::new();

    match solidcraft_io::write_design_container(&s.design()) {
        Ok(b) if b.len() <= MAX_DESIGN_BYTES => put(&folder, &mut files, &mut notes, "design", "design.solidcraft", &b),
        Ok(_) => notes.push(format!("the design is larger than {} MB and was not copied", MAX_DESIGN_BYTES >> 20)),
        Err(e) => notes.push(format!("the design could not be written: {e}")),
    }

    let journal = crash::for_report();
    let mut replayable = false;
    if let Some(j) = &journal {
        notes.extend(j.notes.iter().cloned());
        if let Some(d) = &j.design {
            put(&folder, &mut files, &mut notes, "start", "start.solidcraft", d);
        }
        // This report's own command is in flight while it runs: a replay must not write another.
        if let Some(mut replay) = j.replay.clone() {
            if let Some(cmds) = replay.get_mut("commands").and_then(Value::as_array_mut) {
                cmds.retain(|c| c.get("command").and_then(Value::as_str) != Some("diagnostics.bug_report"));
            }
            let fresh = j.base.as_str() == Some("new");
            if (fresh || files.contains_key("start"))
                && let Ok(b) = serde_json::to_vec_pretty(&replay)
            {
                put(&folder, &mut files, &mut notes, "replay", "replay.json", &b);
                replayable = files.contains_key("replay");
            }
        }
    } else {
        notes.push("no command journal in this run (only the desktop app keeps one): no replay".into());
    }

    match session_log::current().and_then(|p| tail(p, MAX_LOG_BYTES)) {
        Some((b, cut)) => {
            if cut {
                notes.push(format!("log.txt is the last {} MB of this run's log", MAX_LOG_BYTES >> 20));
            }
            put(&folder, &mut files, &mut notes, "log", "log.txt", &b);
        }
        None => notes.push("no log in this run".into()),
    }

    if let Some(png) = &r.screenshot_png {
        put(&folder, &mut files, &mut notes, "screenshot", "screenshot.png", png);
    }

    let features: Vec<Value> = s
        .doc
        .features
        .iter()
        .map(|f| {
            let res = s.model.result(f.id);
            json!({
                "id": f.id,
                "name": f.name,
                "suppressed": f.suppressed,
                "error": res.and_then(|r| r.error.clone()),
                "warning": res.and_then(|r| r.warning.clone()),
            })
        })
        .collect();
    let failing: Vec<&Value> = features.iter().filter(|f| !f["error"].is_null()).collect();

    // Crash reports this run wrote, so the report points at them.
    let crashes: Vec<String> = match (&journal, crash::default_dir()) {
        (Some(j), Some(cdir)) => {
            let since = crash::utc(j.started);
            crash::list(&cdir).into_iter().filter(|c| c.time >= since).map(|c| c.id).collect()
        }
        _ => Vec::new(),
    };

    let dialog = r.dialog.as_ref().map(|d| {
        let text = d.to_string();
        if text.len() <= MAX_DIALOG_BYTES { d.clone() } else { json!(limited(&text, MAX_DIALOG_BYTES)) }
    });
    let (version, commit, built) = journal.as_ref().map_or((env!("CARGO_PKG_VERSION"), "", ""), |j| (j.build.version, j.build.commit, j.build.built));
    let reproduce = match (replayable, files.contains_key("start")) {
        (true, true) => Some(format!("solidcraft-cli reports replay {id}")),
        (true, false) => Some(format!("solidcraft-cli reports replay {id}  (starts from a new design)")),
        _ => None,
    };
    let report = json!({
        "format": FORMAT,
        "id": id,
        "kind": "user_report",
        "time": crash::utc(t),
        "title": limited(r.title.trim(), 300),
        "what_happened": limited(&r.what_happened, MAX_TEXT),
        "expected": limited(&r.expected, MAX_TEXT),
        "steps": limited(&r.steps, MAX_TEXT),
        "status": { "state": "open", "issue": null, "commit": null, "updated": crash::utc(t) },
        "app": { "version": version, "commit": commit, "built": built },
        "system": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "environment": journal.as_ref().map(|j| j.environment.clone()).unwrap_or_default(),
        },
        "context": limited(&r.context, MAX_TEXT),
        "dialog_commands": dialog,
        "design": {
            "name": s.doc.name,
            "saved_as": s.path.as_ref().and_then(|p| Path::new(p).file_name()).map(|n| n.to_string_lossy().into_owned()),
            "unsaved_changes": s.is_dirty(),
            "features": features.len(),
            "failing_features": failing,
        },
        "features": features,
        "recent_commands": journal.as_ref().map(|j| j.recent.clone()).unwrap_or_default(),
        "base": journal.as_ref().map(|j| j.base.clone()).unwrap_or(Value::Null),
        "crash_reports": crashes,
        "files": files,
        "reproduce": reproduce,
        "notes": notes,
    });
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    solidcraft_io::write_atomic(&folder.join("report.json"), &bytes, false).map_err(|e| format!("{}: {e}", folder.display()))?;
    read(dir, &id)
}

fn read_file(folder: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(folder.join("report.json")).ok()?).ok()
}

/// One report's `report.json`, with `path` and the paths of its files (`design_path`,
/// `start_path`, `replay_path`, `log_path`, `screenshot_path`).
pub fn read(dir: &Path, id: &str) -> std::result::Result<Value, String> {
    let missing = || format!("no report `{id}`");
    if !valid_id(id) {
        return Err(missing());
    }
    let folder = dir.join(id);
    let mut v = read_file(&folder).ok_or_else(missing)?;
    v["path"] = json!(folder.to_string_lossy());
    for (k, f) in [
        ("design_path", "design.solidcraft"),
        ("start_path", "start.solidcraft"),
        ("replay_path", "replay.json"),
        ("log_path", "log.txt"),
        ("screenshot_path", "screenshot.png"),
    ] {
        if folder.join(f).is_file() {
            v[k] = json!(folder.join(f).to_string_lossy());
        }
    }
    Ok(v)
}

/// Every report, newest first.
pub fn list(dir: &Path) -> Vec<Summary> {
    ids(dir)
        .into_iter()
        .rev()
        .filter_map(|id| {
            let folder = dir.join(&id);
            let v = read_file(&folder)?;
            let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let status = v.get("status").cloned().unwrap_or(Value::Null);
            Some(Summary {
                time: s(&v, "time"),
                title: s(&v, "title"),
                status: status.get("state").and_then(Value::as_str).unwrap_or("open").to_string(),
                issue: status.get("issue").and_then(Value::as_u64),
                commit: status.get("commit").and_then(Value::as_str).map(str::to_string),
                version: v.get("app").map(|a| s(a, "version")).unwrap_or_default(),
                path: folder.to_string_lossy().into_owned(),
                replayable: folder.join("replay.json").is_file(),
                id,
            })
        })
        .collect()
}

/// Sets a report's status: `state` (`open` or `fixed`), and the issue and commit that track it
/// when given. Returns the report as [`read`] does.
pub fn set_status(dir: &Path, id: &str, state: Option<&str>, issue: Option<u64>, commit: Option<&str>) -> std::result::Result<Value, String> {
    if let Some(st) = state
        && !STATUSES.contains(&st)
    {
        return Err(format!("status `{st}` must be open or fixed"));
    }
    if let Some(c) = commit
        && (c.is_empty() || c.len() > 64 || !c.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(format!("commit `{c}` must be a git hash"));
    }
    let mut v = read(dir, id)?;
    let folder = dir.join(id);
    for k in ["path", "design_path", "start_path", "replay_path", "log_path", "screenshot_path"] {
        if let Some(o) = v.as_object_mut() {
            o.remove(k);
        }
    }
    if !v.get("status").is_some_and(Value::is_object) {
        v["status"] = json!({ "state": "open", "issue": null, "commit": null });
    }
    if let Some(st) = state {
        v["status"]["state"] = json!(st);
    }
    if let Some(n) = issue {
        v["status"]["issue"] = json!(n);
    }
    if let Some(c) = commit {
        v["status"]["commit"] = json!(c);
    }
    v["status"]["updated"] = json!(crash::utc(now()));
    let bytes = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
    solidcraft_io::write_atomic(&folder.join("report.json"), &bytes, false).map_err(|e| format!("{}: {e}", folder.display()))?;
    read(dir, id)
}

/// Deletes one report, or every fixed one. Returns how many went.
pub fn clear(dir: &Path, which: Clear<'_>) -> usize {
    let doomed: Vec<String> = match which {
        Clear::One(id) => ids(dir).into_iter().filter(|i| i == id).collect(),
        Clear::Fixed => list(dir).into_iter().filter(|r| r.status == "fixed").map(|r| r.id).collect(),
    };
    doomed.iter().filter(|i| fs::remove_dir_all(dir.join(i)).is_ok()).count()
}

/// What [`clear`] deletes.
#[derive(Clone, Copy, Debug)]
pub enum Clear<'a> {
    One(&'a str),
    Fixed,
}

#[cfg(test)]
#[path = "bug_report_tests.rs"]
mod tests;
