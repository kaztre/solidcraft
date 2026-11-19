//! Versions of a folder's parts (docs/versions.md): `versions.start` starts keeping them,
//! `versions.status` lists what a check-in would save, `versions.check_in` saves the named parts
//! as a new version, `versions.history` lists the versions (of the folder or one part),
//! `versions.restore` puts parts back, `versions.copy` writes an old version to a new file, and
//! `versions.name` names a version or releases a part revision. None of them change the open
//! design. The history is kept in git's format by `solidcraft_io::versions`, in the folder's own
//! `.git` if it has one and otherwise in SolidCraft's data folder (never inside a synced folder).
//! The Versions tab of the Browser panel drives them.

use std::path::{Path, PathBuf};

use serde_json::Value;
use solidcraft_io::versions::{self, Author, Store};

use super::CommandSpec;
use crate::params::bad;
use crate::schema::versions::{
    VersionChange, VersionEntry, VersionFile, VersionRevision, VersionsCheckInParams, VersionsCheckInReply, VersionsCopyParams, VersionsCopyReply,
    VersionsFolderParams, VersionsHistoryParams, VersionsHistoryReply, VersionsNameParams, VersionsNameReply, VersionsReleaseParams,
    VersionsReleaseReply, VersionsRestoreParams, VersionsRestoreReply, VersionsStatusReply,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const FOLDER_CONTRACT: TypedContract<VersionsFolderParams> = TypedContract::new();
const CHECK_IN_CONTRACT: TypedContract<VersionsCheckInParams> = TypedContract::new();
const HISTORY_CONTRACT: TypedContract<VersionsHistoryParams> = TypedContract::new();
const RESTORE_CONTRACT: TypedContract<VersionsRestoreParams> = TypedContract::new();
const COPY_CONTRACT: TypedContract<VersionsCopyParams> = TypedContract::new();
const NAME_CONTRACT: TypedContract<VersionsNameParams> = TypedContract::new();
const RELEASE_CONTRACT: TypedContract<VersionsReleaseParams> = TypedContract::new();

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 5000;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("versions.status", "Version Status", crate::checked!("versions.status", status))
        .icon("folder")
        .noundo()
        .contract(&FOLDER_CONTRACT)
        .output(output_schema_for::<VersionsStatusReply>)
        .params(
            "folder: a folder whose files keep versions → folder, tracked (false until versions.start), head?, \
             changes: [{path, change: added|changed|deleted, local}], files, truncated. Never changes anything",
        ),
    CommandSpec::new("versions.start", "Keep Versions of a Folder", crate::checked!("versions.start", start))
        .icon("folder")
        .noundo()
        .contract(&FOLDER_CONTRACT)
        .output(output_schema_for::<VersionsStatusReply>)
        .params("folder → as versions.status. Starts an empty history for the folder (kept outside it); nothing is checked in yet"),
    CommandSpec::new("versions.check_in", "Check In", crate::checked!("versions.check_in", check_in))
        .icon("save")
        .noundo()
        .contract(&CHECK_IN_CONTRACT)
        .output(output_schema_for::<VersionsCheckInReply>)
        .params(
            "folder; message: what changed; paths: [files relative to the folder] saved as they are now (a file no longer there is \
             removed); author?: default this computer's user; release?: {path, revision} also releases that part as a revision \
             → commit, short, files: [{path, change}], revision?. Never changes the open design",
        ),
    CommandSpec::new("versions.history", "Version History", crate::checked!("versions.history", history))
        .icon("folder")
        .noundo()
        .contract(&HISTORY_CONTRACT)
        .output(output_schema_for::<VersionsHistoryReply>)
        .params(
            "folder; path?: only the versions that changed this part; limit?: default 200, at most 5000 → folder, commits: [{id, short, \
             message, author, time, files: [{path, change}], versions: [names], revisions: [{path, revision}]}] newest first, more",
        ),
    CommandSpec::new("versions.restore", "Restore Version", crate::checked!("versions.restore", restore))
        .icon("folder")
        .noundo()
        .contract(&RESTORE_CONTRACT)
        .output(output_schema_for::<VersionsRestoreReply>)
        .params(
            "folder; commit: a version's id; path?: one part (default: every part that differs; parts added since stay) → restored: \
             [paths]. The restored files then show as changes, to check in",
        ),
    CommandSpec::new("versions.copy", "Copy Version", crate::checked!("versions.copy", copy))
        .icon("folder")
        .noundo()
        .contract(&COPY_CONTRACT)
        .output(output_schema_for::<VersionsCopyReply>)
        .params(
            "folder; commit; path: the part; to?: a new part in the folder (Save as New Part; its first check-in notes where it came \
             from), default a copy outside the folder to look at → file, path?",
        ),
    CommandSpec::new("versions.name", "Name Version", crate::checked!("versions.name", name))
        .icon("folder")
        .noundo()
        .contract(&NAME_CONTRACT)
        .output(output_schema_for::<VersionsNameReply>)
        .params(
            "folder; commit; version?: a name for the whole folder (v1.3), or path? and revision?: a part's revision (Rev B); author? → tag, commit",
        ),
    CommandSpec::new("versions.release", "Release", crate::checked!("versions.release", release))
        .icon("save")
        .noundo()
        .any_part()
        .contract(&RELEASE_CONTRACT)
        .output(output_schema_for::<VersionsReleaseReply>)
        .params(
            "folder; design: the open design's file relative to the folder, saved and unchanged since, its Revision property \
             `revision`; revision: such as Rev B, not released before; message: what changed; author?; also?: [other files of the \
             folder to check in with it]; dry_run?: only check → \
             revision, ready, blocked: [what keeps it from release], notes: [what a release may still want], files: [the release \
             set, relative to the folder], commit?, short?. Refused, naming every reason, while a feature fails or Check Drawing \
             finds a dimension, balloon or note lost, ambiguous, unmeasurable or on a suppressed feature, a view failed, out of \
             date or empty, or anything off its sheet. Writes Released/<design>/<revision>/: the design as STEP, each drawing as \
             one PDF of its sheets, each sheet metal body's flat pattern as DXF; checks them in with the design as one version \
             released as `revision`, and makes them read-only. Never changes the open design",
        ),
];

/// Where folders without their own `.git` keep their histories.
fn histories() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(solidcraft_io::vfs::temp_dir().join(format!("solidcraft-test-versions-{}", std::process::id())));
    }
    if let Some(d) = solidcraft_geom::env::var_os("VERSIONS_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    Some(crate::recovery::data_dir()?.join("versions"))
}

/// The folder's store: the folder must exist; its history is named after its full path.
fn store(cmd: &str, p: &Value) -> Result<Store> {
    let folder =
        p.get("folder").and_then(Value::as_str).filter(|f| !f.trim().is_empty() && f.len() < 4096).ok_or_else(|| bad(cmd, "`folder` is required"))?;
    let path = Path::new(folder);
    if !path.is_absolute() {
        return Err(bad(cmd, "`folder` must be a full path"));
    }
    let work = std::fs::canonicalize(path).map_err(|e| bad(cmd, format!("{folder}: {e}")))?;
    if !work.is_dir() {
        return Err(bad(cmd, format!("{folder} isn't a folder")));
    }
    let root = histories().ok_or_else(|| bad(cmd, "this computer has no folder for SolidCraft's data"))?;
    let key = solidcraft_doc::DesignLink::file_digest(work.to_string_lossy().as_bytes());
    let name = work.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).take(40).collect();
    let history = root.join(format!("{name}-{}.git", key.get(..12).unwrap_or(&key)));
    Ok(Store::locate(&work, &history))
}

fn text<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn commit_id<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    text(p, "commit").ok_or_else(|| bad(cmd, "`commit` is required: a version's id from versions.history"))
}

fn author(p: &Value) -> Author {
    let user =
        text(p, "author").map(str::to_string).or_else(|| std::env::var("USER").ok()).or_else(|| std::env::var("USERNAME").ok()).unwrap_or_default();
    let host = user.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).collect::<String>().to_lowercase();
    Author::new(&user, &format!("{}@solidcraft.local", if host.is_empty() { "someone" } else { &host }))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn failed(cmd: &str) -> impl Fn(versions::VersionsError) -> crate::EngineError + '_ {
    move |e| bad(cmd, e.to_string())
}

fn status_reply(cmd: &str, s: &Store) -> Result<Value> {
    let folder = s.work.to_string_lossy().into_owned();
    let reply = if s.exists() {
        let st = s.status().map_err(failed(cmd))?;
        VersionsStatusReply {
            folder,
            tracked: true,
            head: st.head,
            changes: st.changes.into_iter().map(|c| VersionChange { path: c.path, change: c.change.as_str().into(), local: c.local }).collect(),
            files: st.files as u64,
            truncated: st.truncated,
        }
    } else {
        VersionsStatusReply { folder, tracked: false, head: None, changes: Vec::new(), files: 0, truncated: false }
    };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

fn status(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.status";
    status_reply(cmd, &store(cmd, p)?)
}

fn start(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.start";
    let s = store(cmd, p)?;
    s.start().map_err(failed(cmd))?;
    status_reply(cmd, &s)
}

fn check_in(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.check_in";
    let s = store(cmd, p)?;
    let message = text(p, "message").ok_or_else(|| bad(cmd, "`message` is required: say what changed"))?;
    let paths: Vec<String> = p
        .get("paths")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| bad(cmd, "`paths` must be a list of files in the folder"))?
        .iter()
        .map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad(cmd, "`paths` must be a list of files in the folder")))
        .collect::<Result<_>>()?;
    let release = match p.get("release") {
        None | Some(Value::Null) => None,
        Some(r) => {
            let path = text(r, "path").ok_or_else(|| bad(cmd, "`release.path` is required"))?;
            let revision = text(r, "revision").ok_or_else(|| bad(cmd, "`release.revision` is required, such as Rev B"))?;
            if !paths.iter().any(|x| x == path) {
                return Err(bad(cmd, "`release.path` must be one of `paths`"));
            }
            Some((path.to_string(), revision.to_string()))
        }
    };
    let who = author(p);
    let c = s.check_in(&paths, message, &who, now()).map_err(failed(cmd))?;
    let (log, _) = s.log(None, 1).map_err(failed(cmd))?;
    let files = log.into_iter().next().map(|e| e.files).unwrap_or_default();
    let revision = match release {
        Some((path, revision)) => {
            s.name(&c.id, None, Some((&path, &revision)), &who, now()).map_err(failed(cmd))?;
            Some(revision)
        }
        None => None,
    };
    let reply = VersionsCheckInReply {
        short: versions::short(&c.id).into(),
        commit: c.id,
        files: files.into_iter().map(|(path, change)| VersionFile { path, change: change.as_str().into() }).collect(),
        revision,
    };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

fn history(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.history";
    let s = store(cmd, p)?;
    let limit = match p.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(v) => {
            v.as_u64().filter(|n| *n > 0).ok_or_else(|| bad(cmd, "`limit` must be a whole number of versions, 1 or more"))?.min(MAX_LIMIT as u64)
                as usize
        }
    };
    let path = text(p, "path");
    if let Some(path) = path {
        versions::check_path(path).map_err(failed(cmd))?;
    }
    let folder = s.work.to_string_lossy().into_owned();
    if !s.exists() {
        return serde_json::to_value(VersionsHistoryReply { folder, commits: Vec::new(), more: false }).map_err(|e| bad(cmd, e.to_string()));
    }
    let (log, more) = s.log(path, limit).map_err(failed(cmd))?;
    let tags = s.tags().map_err(failed(cmd))?;
    let commits = log
        .into_iter()
        .map(|e| {
            let named = tags.iter().filter(|t| t.commit == e.commit.id);
            VersionEntry {
                short: versions::short(&e.commit.id).into(),
                message: e.commit.message.clone(),
                author: e.commit.author.name.clone(),
                time: e.commit.time,
                files: e.files.into_iter().map(|(path, change)| VersionFile { path, change: change.as_str().into() }).collect(),
                versions: named.clone().filter_map(|t| t.version.clone()).collect(),
                revisions: named.filter_map(|t| Some(VersionRevision { path: t.part.clone()?, revision: t.revision.clone()? })).collect(),
                id: e.commit.id,
            }
        })
        .collect();
    serde_json::to_value(VersionsHistoryReply { folder, commits, more }).map_err(|e| bad(cmd, e.to_string()))
}

fn restore(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.restore";
    let s = store(cmd, p)?;
    let commit = commit_id(cmd, p)?;
    let restored = s.restore(commit, text(p, "path")).map_err(failed(cmd))?;
    serde_json::to_value(VersionsRestoreReply { restored }).map_err(|e| bad(cmd, e.to_string()))
}

fn copy(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.copy";
    let s = store(cmd, p)?;
    let commit = commit_id(cmd, p)?;
    let path = text(p, "path").ok_or_else(|| bad(cmd, "`path` is required: the part"))?;
    versions::check_path(path).map_err(failed(cmd))?;
    let reply = match text(p, "to") {
        Some(to) => {
            versions::check_path(to).map_err(failed(cmd))?;
            if !versions::is_kept(to) {
                return Err(bad(cmd, "`to` must be a file name versions keep, not a lock, backup or temporary file"));
            }
            let dest = s.work.join(to);
            let note = format!("Copied from {path} ({})", versions::short(commit));
            s.copy_out(commit, path, &dest, Some((to, &note))).map_err(failed(cmd))?;
            VersionsCopyReply { file: dest.to_string_lossy().into_owned(), path: Some(to.to_string()) }
        }
        None => {
            let root = histories().ok_or_else(|| bad(cmd, "this computer has no folder for SolidCraft's data"))?;
            let file = Path::new(path);
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = file.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let short = versions::short(commit);
            let dest = root.join("opened").join(short).join(format!("{stem} ({short}).{ext}"));
            if !dest.is_file() {
                s.copy_out(commit, path, &dest, None).map_err(failed(cmd))?;
            }
            VersionsCopyReply { file: dest.to_string_lossy().into_owned(), path: None }
        }
    };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

fn name(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.name";
    let s = store(cmd, p)?;
    let commit = commit_id(cmd, p)?;
    let part = match (text(p, "path"), text(p, "revision")) {
        (Some(path), Some(rev)) => Some((path, rev)),
        (None, None) => None,
        _ => return Err(bad(cmd, "a revision needs both `path` and `revision`")),
    };
    let tag = s.name(commit, text(p, "version"), part, &author(p), now()).map_err(failed(cmd))?;
    serde_json::to_value(VersionsNameReply { tag: tag.name, commit: tag.commit }).map_err(|e| bad(cmd, e.to_string()))
}

/// Check Drawing findings that keep a drawing from release; the others are noted.
const BLOCKING: &[&str] = &["lost", "ambiguous", "unmeasurable", "suppressed", "failed", "out_of_date", "nothing_shown", "outside_sheet"];

/// `name` as one file name: no folder separators or characters a file system refuses.
fn file_part(name: &str) -> String {
    let out: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '-' } else { c }).collect();
    let out = out.trim().trim_matches('.').trim().to_string();
    if out.is_empty() { "part".into() } else { out.chars().take(120).collect() }
}

fn nested(s: &mut Session, id: &str, p: Value) -> Result<Value> {
    let spec = super::find_command(id).ok_or_else(|| crate::EngineError::UnknownCommand(id.to_string()))?;
    spec.run_nested(s, &p)
}

/// The design's drawings, each checked: what blocks its release and what it may still want.
fn check_drawings(s: &mut Session, blocked: &mut Vec<String>, notes: &mut Vec<String>) -> Result<()> {
    let drawings: Vec<(u64, String)> = s.doc.drawings.iter().map(|d| (d.id, d.name.clone())).collect();
    for (id, name) in drawings {
        nested(s, "drawing.open", serde_json::json!({"drawing": solidcraft_doc::ids::encode(id)}))?;
        let check = nested(s, "drawing.check", serde_json::json!({}))?;
        for f in check["findings"].as_array().into_iter().flatten() {
            let kind = f["kind"].as_str().unwrap_or("");
            let why = f["why"].as_str().unwrap_or(kind);
            let what = match f["object"]["kind"].as_str() {
                Some(object) => format!("drawing {name}: {} {object}: {why}", kind.replace('_', " ")),
                None => format!("drawing {name}: {why}"),
            };
            if BLOCKING.contains(&kind) { blocked.push(what) } else { notes.push(what) }
        }
        for c in check["coverage"].as_array().into_iter().flatten().filter(|c| c["state"] == "not_covered") {
            notes.push(format!("drawing {name}: nothing dimensions feature {}", c["name"].as_str().unwrap_or("?")));
        }
    }
    Ok(())
}

/// Write the release set into `dir`: the design as STEP, each drawing's sheets as one PDF, each
/// sheet metal body's flat pattern as DXF. The names, relative to `dir`.
fn write_release_set(s: &mut Session, dir: &Path, stem: &str, revision: &str) -> Result<Vec<String>> {
    let mut names = vec![format!("{stem} {revision}.step")];
    let at = |n: &str| dir.join(n).to_string_lossy().into_owned();
    nested(s, "file.export", serde_json::json!({"path": at(&names[0]), "format": "step"}))?;
    let drawings: Vec<(u64, String)> = s.doc.drawings.iter().map(|d| (d.id, d.name.clone())).collect();
    for (id, name) in drawings {
        let file = format!("{} {revision}.pdf", file_part(&name));
        nested(s, "drawing.open", serde_json::json!({"drawing": solidcraft_doc::ids::encode(id)}))?;
        nested(s, "drawing.export", serde_json::json!({"all_sheets": true, "format": "pdf", "path": at(&file)}))?;
        names.push(file);
    }
    s.open_drawing = None;
    let sheets: Vec<String> = s.model.state().sheets.iter().map(|sh| sh.body.clone()).collect();
    for body in sheets {
        let file = format!("{stem} {} {revision} flat.dxf", file_part(&body));
        nested(s, "sheet.export_dxf", serde_json::json!({"body": body, "path": at(&file)}))?;
        names.push(file);
    }
    Ok(names)
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "versions.release";
    let store = store(cmd, p)?;
    if !store.exists() {
        return Err(bad(cmd, "the folder doesn't keep versions yet: start keeping them first (versions.start)"));
    }
    let design = text(p, "design").ok_or_else(|| bad(cmd, "`design` is required: the design's file, relative to the folder"))?;
    versions::check_path(design).map_err(failed(cmd))?;
    let revision = text(p, "revision").ok_or_else(|| bad(cmd, "`revision` is required, such as Rev B"))?;
    if revision.len() > 64 || file_part(revision) != revision {
        return Err(bad(cmd, "`revision` must be a short name that can be part of a file name, such as Rev B"));
    }
    let message = text(p, "message").ok_or_else(|| bad(cmd, "`message` is required: say what changed"))?;
    let dry_run = p.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
    let also: Vec<String> = p.get("also").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    for other in &also {
        versions::check_path(other).map_err(failed(cmd))?;
    }
    let stem = file_part(&Path::new(design).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let folder = format!("Released/{stem}/{revision}");
    let dir = store.work.join(&folder);

    let mut blocked = Vec::new();
    let mut notes = Vec::new();
    if !store.work.join(design).is_file() {
        blocked.push(format!("{design} isn't in the folder: save the design there first"));
    }
    if s.is_dirty() {
        blocked.push("the design has changes that aren't saved: save it first".into());
    }
    if s.doc.metadata.text("revision").map(str::trim) != Some(revision) {
        blocked.push(format!("the design's Revision property isn't {revision}: set it and save the design first"));
    }
    let tags = store.tags().map_err(failed(cmd))?;
    if dir.exists() || tags.iter().any(|t| t.part.as_deref() == Some(design) && t.revision.as_deref() == Some(revision)) {
        blocked.push(format!("{design} was released as {revision} already: release the next revision"));
    }
    for r in &s.model.results {
        if let Some(e) = &r.error {
            blocked.push(format!("feature {} fails: {e}", r.name));
        }
    }
    let open = s.open_drawing;
    let checked = check_drawings(s, &mut blocked, &mut notes);
    s.open_drawing = open;
    checked?;

    let mut files = vec![design.to_string(), format!("{folder}/{stem} {revision}.step")];
    files.extend(s.doc.drawings.iter().map(|d| format!("{folder}/{} {revision}.pdf", file_part(&d.name))));
    files.extend(s.model.state().sheets.iter().map(|sh| format!("{folder}/{stem} {} {revision} flat.dxf", file_part(&sh.body))));
    let reply = |ready, blocked, notes, files, commit: Option<String>| VersionsReleaseReply {
        revision: revision.to_string(),
        ready,
        blocked,
        notes,
        files,
        short: commit.as_deref().map(|c| versions::short(c).to_string()),
        commit,
    };
    if !blocked.is_empty() && !dry_run {
        return Err(bad(cmd, format!("{design} isn't released as {revision}: {}", blocked.join("; "))));
    }
    if dry_run {
        let ready = blocked.is_empty();
        return serde_json::to_value(reply(ready, blocked, notes, files, None)).map_err(|e| bad(cmd, e.to_string()));
    }

    // Written beside the release folder first, and moved into place only once all of it is.
    let staging = store.work.join(format!("Released/{stem}/.{revision} (being released)"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| bad(cmd, format!("{}: {e}", staging.display())))?;
    let written = write_release_set(s, &staging, &stem, revision);
    s.open_drawing = open;
    let names = match written {
        Ok(names) => names,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(bad(cmd, format!("the release set couldn't be written: {e}")));
        }
    };
    if let Err(e) = std::fs::rename(&staging, &dir) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(bad(cmd, format!("{}: {e}", dir.display())));
    }
    let mut paths = vec![design.to_string()];
    paths.extend(names.iter().map(|n| format!("{folder}/{n}")));
    for other in also {
        if !paths.contains(&other) {
            paths.push(other);
        }
    }
    let who = author(p);
    let note = format!("{message}\n\nReleased as {revision}: {}", names.join(", "));
    let checked_in =
        store.check_in(&paths, &note, &who, now()).and_then(|c| store.name(&c.id, None, Some((design, revision)), &who, now()).map(|_| c));
    let c = match checked_in {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(bad(cmd, e.to_string()));
        }
    };
    // A released file is never written over: the next change is the next revision.
    for n in &names {
        let file = dir.join(n);
        if let Ok(meta) = std::fs::metadata(&file) {
            let mut perms = meta.permissions();
            perms.set_readonly(true);
            let _ = std::fs::set_permissions(&file, perms);
        }
    }
    serde_json::to_value(reply(true, Vec::new(), notes, paths, Some(c.id))).map_err(|e| bad(cmd, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("solidcraft-versions-cmd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("PARTS")).unwrap();
        std::fs::canonicalize(d).unwrap()
    }

    #[test]
    fn a_folder_checks_in_a_part_releases_it_and_restores_it() {
        let dir = folder("flow");
        let f = dir.to_str().unwrap();
        std::fs::write(dir.join("PARTS/Clip.solidcraft"), "clip one").unwrap();
        let mut s = Session::default();
        let st = s.execute("versions.status", &json!({"folder": f})).unwrap();
        assert_eq!(st["tracked"], false, "{st}");
        let st = s.execute("versions.start", &json!({"folder": f})).unwrap();
        assert_eq!(st["tracked"], true);
        assert_eq!(st["changes"], json!([{"path": "PARTS/Clip.solidcraft", "change": "added", "local": true}]));
        let r = s
            .execute(
                "versions.check_in",
                &json!({"folder": f, "message": "Clip", "paths": ["PARTS/Clip.solidcraft"], "author": "Sam", "release": {"path": "PARTS/Clip.solidcraft", "revision": "Rev A"}}),
            )
            .unwrap();
        let first = r["commit"].as_str().unwrap().to_string();
        assert_eq!(r["revision"], "Rev A");
        assert_eq!(r["files"], json!([{"path": "PARTS/Clip.solidcraft", "change": "added"}]));
        assert!(!dir.join(".git").exists(), "the history is kept outside the folder");

        std::fs::write(dir.join("PARTS/Clip.solidcraft"), "clip two, longer").unwrap();
        s.execute("versions.check_in", &json!({"folder": f, "message": "Longer clip", "paths": ["PARTS/Clip.solidcraft"]})).unwrap();
        let h = s.execute("versions.history", &json!({"folder": f, "path": "PARTS/Clip.solidcraft"})).unwrap();
        let commits = h["commits"].as_array().unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0]["message"], "Longer clip");
        assert_eq!(commits[1]["revisions"], json!([{"path": "PARTS/Clip.solidcraft", "revision": "Rev A"}]));
        assert_eq!(commits[1]["author"], "Sam");

        let n = s.execute("versions.name", &json!({"folder": f, "commit": first, "version": "v1.0"})).unwrap();
        assert_eq!(n["tag"], "v1.0");
        let c = s.execute("versions.copy", &json!({"folder": f, "commit": first, "path": "PARTS/Clip.solidcraft"})).unwrap();
        assert_eq!(std::fs::read_to_string(c["file"].as_str().unwrap()).unwrap(), "clip one");
        let c = s
            .execute("versions.copy", &json!({"folder": f, "commit": first, "path": "PARTS/Clip.solidcraft", "to": "PARTS/Clip short.solidcraft"}))
            .unwrap();
        assert_eq!(c["path"], "PARTS/Clip short.solidcraft");

        let r = s.execute("versions.restore", &json!({"folder": f, "commit": first, "path": "PARTS/Clip.solidcraft"})).unwrap();
        assert_eq!(r["restored"], json!(["PARTS/Clip.solidcraft"]));
        assert_eq!(std::fs::read_to_string(dir.join("PARTS/Clip.solidcraft")).unwrap(), "clip one");
        let st = s.execute("versions.status", &json!({"folder": f})).unwrap();
        assert_eq!(st["changes"].as_array().unwrap().len(), 2, "{st}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A release checks the part and its drawings, writes the release set (the STEP and each
    /// drawing's PDF) beside it, checks them in with the part as the revision and makes the set
    /// read-only; the same revision again, or a part with a failing feature, is refused, naming
    /// why (fork).
    #[test]
    fn a_release_writes_its_set_checks_it_in_and_refuses_what_isnt_ready() {
        let dir = folder("release");
        let f = dir.to_str().unwrap();
        let design = dir.join("PARTS/Plate.solidcraft");
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length": 40, "width": 20, "height": 5, "body_name": "Plate"})).unwrap();
        s.execute("drawing.create", &json!({"name": "Plate drawing", "standard_views": true})).unwrap();
        s.execute("drawing.close", &json!({})).unwrap();
        s.execute("document.metadata.set", &json!({"values": {"revision": "Rev A"}})).unwrap();
        s.execute("file.save", &json!({"path": design})).unwrap();
        s.execute("versions.start", &json!({"folder": f})).unwrap();
        let ask = json!({"folder": f, "design": "PARTS/Plate.solidcraft", "revision": "Rev A", "message": "First release", "author": "Sam"});
        let mut dry = ask.clone();
        dry["dry_run"] = json!(true);
        let check = s.execute("versions.release", &dry).unwrap();
        assert_eq!(check["ready"], true, "{check}");
        assert!(!dir.join("Released").exists(), "a dry run writes nothing");

        let r = s.execute("versions.release", &ask).unwrap();
        let files: Vec<&str> = r["files"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(files, ["PARTS/Plate.solidcraft", "Released/Plate/Rev A/Plate Rev A.step", "Released/Plate/Rev A/Plate drawing Rev A.pdf"], "{r}");
        for (file, start) in [(files[1], &b"ISO-10303-21"[..]), (files[2], &b"%PDF"[..])] {
            let path = dir.join(file);
            assert!(std::fs::read(&path).unwrap().starts_with(start), "{file}");
            assert!(std::fs::metadata(&path).unwrap().permissions().readonly(), "{file} is read-only");
        }
        let h = s.execute("versions.history", &json!({"folder": f})).unwrap();
        assert_eq!(h["commits"][0]["revisions"], json!([{"path": "PARTS/Plate.solidcraft", "revision": "Rev A"}]), "{h}");
        assert_eq!(h["commits"][0]["files"].as_array().unwrap().len(), 3, "{h}");
        assert_eq!(s.execute("versions.status", &json!({"folder": f})).unwrap()["changes"], json!([]));
        let again = s.execute("versions.release", &ask).unwrap_err().to_string();
        assert!(again.contains("released as Rev A already"), "{again}");

        // A failing feature keeps Rev B from release, and nothing is written.
        s.execute("solid.box", &json!({"length": 5, "width": 5, "height": 5, "corner": [0, 0, 0], "operation": "cut", "name": "Pocket"})).unwrap();
        s.execute("solid.fillet", &json!({"edges": [[2.5, 5, 5]], "radius": 1, "name": "Round"})).unwrap();
        s.execute("timeline.suppress", &json!({"feature": "Pocket", "suppressed": true})).unwrap();
        s.execute("document.metadata.set", &json!({"values": {"revision": "Rev B"}})).unwrap();
        s.execute("file.save", &json!({"path": design})).unwrap();
        let mut next = ask.clone();
        next["revision"] = json!("Rev B");
        let refused = s.execute("versions.release", &next).unwrap_err().to_string();
        assert!(refused.contains("feature Round fails"), "{refused}");
        assert!(!dir.join("Released/Plate/Rev B").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_params_are_refused() {
        let dir = folder("hostile");
        let f = dir.to_str().unwrap();
        let mut s = Session::default();
        s.execute("versions.start", &json!({"folder": f})).unwrap();
        for (id, params) in [
            ("versions.status", json!({"folder": "relative/folder"})),
            ("versions.status", json!({"folder": dir.join("missing").to_str().unwrap()})),
            ("versions.check_in", json!({"folder": f, "message": "x", "paths": ["../../etc/passwd"]})),
            ("versions.check_in", json!({"folder": f, "message": " ", "paths": ["PARTS/A.solidcraft"]})),
            (
                "versions.check_in",
                json!({"folder": f, "message": "x", "paths": ["PARTS/A.solidcraft"], "release": {"path": "PARTS/B.solidcraft", "revision": "A"}}),
            ),
            ("versions.restore", json!({"folder": f, "commit": "../../../x"})),
            ("versions.copy", json!({"folder": f, "commit": "0".repeat(40), "path": "PARTS/A.solidcraft", "to": "../A.solidcraft"})),
            ("versions.name", json!({"folder": f, "commit": "0".repeat(40), "path": "PARTS/A.solidcraft"})),
            ("versions.history", json!({"folder": f, "path": "/etc/passwd"})),
        ] {
            assert!(s.execute(id, &params).is_err(), "{id} {params}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
