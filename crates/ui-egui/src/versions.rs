//! The Versions tab of the Browser panel (docs/versions.md): check-in, history, revisions and
//! restore for the files in a project folder, through the `versions.*` commands. It
//! follows the design in front: **This part** (or **This assembly**: the design and its linked
//! parts in the folder) or **Whole folder**. A design outside every folder that keeps versions
//! offers to keep versions of its folder. The commands run beside the window; the panel shows
//! what they last found and looks again every few seconds while it is open.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::Receiver;

use serde_json::{Value, json};

use crate::SolveApp;
use crate::ui_kit;
use solidcraft_theme::Tokens;
use solidcraft_ui_text::{format, tr};

/// How often an open Versions tab looks at the folder again, in seconds.
const EVERY: f64 = 3.0;
/// Versions the history lists.
const HISTORY: u64 = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// The design in front, and its linked parts in the folder.
    #[default]
    Design,
    /// Everything in the folder.
    Folder,
}

/// A question asked under the picked version before acting on it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Confirm {
    RestorePart(String),
    RestoreAll,
    Name,
    Release(String),
    NewPart(String),
}

/// What to do when a command run beside the window finishes.
enum After {
    Started(String),
    CheckedIn { message: String },
    Restored,
    Open,
    NewPart,
    Named,
    Released { name: String, revision: String },
}

/// A check-in waiting for the design in front to finish saving.
struct Queued {
    folder: String,
    message: String,
    paths: Vec<String>,
    release: Option<(String, String)>,
}

struct Found {
    folder: String,
    status: Result<Value, String>,
    history: Option<Result<Value, String>>,
}

/// The Versions tab's state.
#[derive(Default)]
pub struct Versions {
    /// The tab is showing instead of the Browser's rows.
    pub open: bool,
    pub scope: Scope,
    /// The folder last looked at, its status and history.
    folder: Option<String>,
    status: Option<Result<Value, String>>,
    history: Option<Result<Value, String>>,
    looking: Option<Receiver<Found>>,
    next: f64,
    /// The history must be read again (something was checked in or named).
    stale: bool,
    acting: Option<(After, Receiver<Result<Value, String>>)>,
    queued: Option<Queued>,
    /// The Whole folder view's folder when the design in front is in none.
    picked: usize,
    message: String,
    /// Changes left out of the next check-in.
    left_out: BTreeSet<String>,
    release: bool,
    selected: Option<String>,
    confirm: Option<Confirm>,
    name: String,
    editing_name: bool,
}

impl Versions {
    /// Look at the folder again on the next frame.
    pub fn soon(&mut self) {
        self.next = 0.0;
    }
}

/// `file` relative to `folder`, `/` between folders, when it is inside it.
fn relative(folder: &str, file: &str) -> Option<String> {
    let rest = Path::new(file).strip_prefix(folder).ok()?;
    let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Does SolidCraft open this file itself: a design, a CAD file or a mesh?
fn opens_here(file: &str) -> bool {
    use solidcraft_engine::io::{is_design_path, is_mesh_path, is_step_path};
    is_design_path(file) || is_step_path(file) || is_mesh_path(file)
}

/// The folder keeping versions that holds `file` (the innermost one).
fn folder_of(app: &SolveApp, file: &str) -> Option<String> {
    app.preferences.versions_folders.iter().filter(|f| Path::new(file).starts_with(f.as_str())).max_by_key(|f| f.len()).cloned()
}

fn file_name(path: &str) -> String {
    Path::new(path).file_stem().map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned())
}

/// The folder shown, the front design relative to it, and the files This part or This
/// assembly covers.
struct View {
    folder: Option<String>,
    design: Option<String>,
    files: BTreeSet<String>,
    assembly: bool,
}

fn view(app: &mut SolveApp) -> View {
    let path = app.session.path.clone().filter(|p| !p.is_empty());
    let own = path.as_deref().and_then(|p| folder_of(app, p));
    let folder = match (&own, app.versions.scope) {
        (Some(f), _) => Some(f.clone()),
        (None, Scope::Folder) => {
            let folders = &app.preferences.versions_folders;
            folders.get(app.versions.picked.min(folders.len().saturating_sub(1))).cloned()
        }
        (None, Scope::Design) => None,
    };
    let design = match (&own, &path) {
        (Some(f), Some(p)) => relative(f, p),
        _ => None,
    };
    let mut files: BTreeSet<String> = design.iter().cloned().collect();
    let assembly = !app.session.doc.links.is_empty();
    if let (Some(f), true) = (&own, assembly)
        && let Ok(r) = app.session.execute("component.links", &json!({}))
    {
        for l in r["links"].as_array().into_iter().flatten() {
            if let Some(rel) = l["file"].as_str().and_then(|file| relative(f, file)) {
                files.insert(rel);
            }
        }
    }
    View { folder, design, files, assembly }
}

fn run_beside(id: &'static str, params: Value) -> Option<Receiver<Result<Value, String>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("versions".into()).spawn(move || {
        let mut s = solidcraft_engine::Session::default();
        let _ = tx.send(s.execute(id, &params).map_err(|e| e.to_string()));
    });
    spawned.ok().map(|_| rx)
}

fn act(app: &mut SolveApp, after: After, id: &'static str, params: Value) {
    if app.versions.acting.is_some() {
        return;
    }
    match run_beside(id, params) {
        Some(rx) => app.versions.acting = Some((after, rx)),
        None => app.set_status(format("ui.versions_failed", &[("error", "no thread to run on")]), true),
    }
}

/// Look at the folder beside the window: its status, and its history when it changed.
fn look(app: &mut SolveApp, folder: &str) {
    let changed_folder = app.versions.folder.as_deref() != Some(folder);
    if changed_folder {
        app.versions.status = None;
        app.versions.history = None;
        app.versions.selected = None;
        app.versions.confirm = None;
        app.versions.left_out.clear();
    }
    let head = app.versions.status.as_ref().and_then(|s| s.as_ref().ok()).and_then(|s| s["head"].as_str().map(str::to_string));
    let force = changed_folder || app.versions.stale || app.versions.history.is_none();
    app.versions.folder = Some(folder.to_string());
    app.versions.stale = false;
    let folder = folder.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("versions-look".into()).spawn(move || {
        let mut s = solidcraft_engine::Session::default();
        let status = s.execute("versions.status", &json!({"folder": folder})).map_err(|e| e.to_string());
        let now = status.as_ref().ok().and_then(|v| v["head"].as_str().map(str::to_string));
        let history =
            (force || now != head).then(|| s.execute("versions.history", &json!({"folder": folder, "limit": HISTORY})).map_err(|e| e.to_string()));
        let _ = tx.send(Found { folder, status, history });
    });
    if spawned.is_ok() {
        app.versions.looking = Some(rx);
    }
}

/// Take up finished work, start a queued check-in, and look again when it's time.
fn poll(app: &mut SolveApp, ctx: &egui::Context, folder: Option<&str>) {
    if let Some(rx) = &app.versions.looking {
        match rx.try_recv() {
            Ok(found) => {
                app.versions.looking = None;
                if app.versions.folder.as_deref() == Some(found.folder.as_str()) {
                    app.versions.status = Some(found.status);
                    if let Some(h) = found.history {
                        app.versions.history = Some(h);
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => app.versions.looking = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    let finished = match &app.versions.acting {
        Some((_, rx)) => match rx.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("stopped".into())),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        },
        None => None,
    };
    if let Some(result) = finished
        && let Some((after, _)) = app.versions.acting.take()
    {
        done(app, after, result);
    }
    if app.saving.is_none()
        && app.versions.acting.is_none()
        && let Some(q) = app.versions.queued.take()
    {
        check_in_now(app, q);
    }
    let now = ctx.input(|i| i.time);
    if let Some(folder) = folder
        && app.versions.looking.is_none()
        && (app.versions.folder.as_deref() != Some(folder) || now >= app.versions.next)
    {
        app.versions.next = now + EVERY;
        look(app, folder);
    }
    let busy = app.versions.looking.is_some() || app.versions.acting.is_some() || app.versions.queued.is_some();
    ctx.request_repaint_after(std::time::Duration::from_millis(if busy { 100 } else { 1000 }));
}

fn done(app: &mut SolveApp, after: After, result: Result<Value, String>) {
    app.versions.stale = true;
    app.versions.soon();
    let r = match result {
        Ok(r) => r,
        Err(e) => {
            app.set_status(format("ui.versions_failed", &[("error", &e)]), true);
            return;
        }
    };
    match after {
        After::Started(folder) => {
            if !app.preferences.versions_folders.contains(&folder) {
                app.preferences.versions_folders.push(folder.clone());
            }
            app.set_status(format("ui.versions_started", &[("folder", &file_name(&folder))]), false);
        }
        After::CheckedIn { message } => {
            let count = r["files"].as_array().map_or(0, Vec::len);
            let key = if count == 1 { "ui.versions_checked_in_one" } else { "ui.versions_checked_in_other" };
            app.set_status(format(key, &[("count", &count.to_string()), ("message", &message)]), false);
            app.versions.message.clear();
            app.versions.release = false;
            app.versions.left_out.clear();
        }
        After::Restored => {
            let restored: Vec<String> = r["restored"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            // The design in front, if it was one of them, shows what was put back.
            if let (Some(folder), Some(path)) = (app.versions.folder.clone(), app.session.path.clone())
                && relative(&folder, &path).is_some_and(|rel| restored.contains(&rel))
                && !app.session.is_dirty()
            {
                let _ = app.run("doc.open", json!({"path": path}));
            }
            let key = if restored.len() == 1 { "ui.versions_restored_one" } else { "ui.versions_restored_other" };
            app.set_status(format(key, &[("count", &restored.len().to_string())]), false);
            app.versions.selected = None;
        }
        After::Open | After::NewPart => {
            if let Some(file) = r["file"].as_str() {
                if opens_here(file) {
                    app.open_path_with(file, &serde_json::Map::new());
                } else if let Some(open) = &app.services.open_file {
                    // A drawing, document or KiCad file: in its own program.
                    open(file);
                } else if let Some(show) = &app.services.show_in_folder {
                    show(file);
                }
            }
            if matches!(after, After::NewPart) {
                app.set_status(format("ui.versions_copied", &[("name", &file_name(r["file"].as_str().unwrap_or("")))]), false);
            }
        }
        After::Named => app.set_status(format("ui.versions_named", &[("name", r["tag"].as_str().unwrap_or(""))]), false),
        After::Released { name, revision } => app.set_status(format("ui.versions_released", &[("name", &name), ("revision", &revision)]), false),
    }
}

fn check_in_now(app: &mut SolveApp, q: Queued) {
    let mut params = json!({"folder": q.folder, "message": q.message, "paths": q.paths});
    if let Some((path, revision)) = q.release {
        params["release"] = json!({"path": path, "revision": revision});
    }
    if !app.preferences.versions_author.trim().is_empty() {
        params["author"] = json!(app.preferences.versions_author.trim());
    }
    act(app, After::CheckedIn { message: q.message }, "versions.check_in", params);
}

/// The revision after the ones `path` has: Rev A, then the next letter.
fn next_revision(history: Option<&Value>, path: &str) -> String {
    let last = history
        .and_then(|h| h["commits"].as_array())
        .into_iter()
        .flatten()
        .flat_map(|c| c["revisions"].as_array().into_iter().flatten())
        .filter(|r| r["path"] == path)
        .filter_map(|r| r["revision"].as_str()?.trim().strip_prefix("Rev ")?.chars().next().filter(char::is_ascii_uppercase))
        .max();
    let letter = match last {
        Some('Z') => return "Rev AA".into(),
        Some(c) => char::from(c as u8 + 1),
        None => 'A',
    };
    format!("Rev {letter}")
}

/// "3 min ago", "yesterday", or the date.
fn when(time: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let ago = (now - time).max(0);
    match ago {
        0..60 => tr("ui.versions_just_now").to_string(),
        60..3600 => format("ui.versions_minutes", &[("count", &(ago / 60).to_string())]),
        3600..86400 => format("ui.versions_hours", &[("count", &(ago / 3600).to_string())]),
        86400..172800 => tr("ui.versions_yesterday").to_string(),
        172800..604800 => format("ui.versions_days", &[("count", &(ago / 86400).to_string())]),
        _ => {
            // Days to a calendar date (proleptic Gregorian), so no clock crate is needed.
            let z = time.div_euclid(86400) + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + i64::from(m <= 2);
            std::format!("{y}-{m:02}-{d:02}")
        }
    }
}

fn change_word(change: &str) -> String {
    tr(match change {
        "added" => "ui.versions_added",
        "deleted" => "ui.versions_deleted",
        _ => "ui.versions_changed",
    })
    .to_string()
}

/// The tab's contents, below the Browser panel's header.
pub fn panel(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    if cfg!(target_arch = "wasm32") {
        ui_kit::text(ui, tr("ui.versions_desktop_only"), ui_kit::Text::Hint("versions"));
        return;
    }
    let v = view(app);
    poll(app, ui.ctx(), v.folder.as_deref());
    ui.add_space(t.space_3);
    let part_label = tr(if v.assembly { "ui.versions_scope_assembly" } else { "ui.versions_scope_part" });
    let mut scope = usize::from(app.versions.scope == Scope::Folder);
    if ui_kit::segmented(ui, &mut scope, &[part_label, tr("ui.versions_scope_folder")], Default::default()) {
        app.versions.scope = if scope == 1 { Scope::Folder } else { Scope::Design };
        app.versions.selected = None;
        app.versions.confirm = None;
    }
    ui.add_space(t.space_3);
    let Some(folder) = v.folder.clone() else {
        not_kept(app, ui);
        return;
    };
    egui::ScrollArea::vertical().id_salt("sc_versions_scroll").auto_shrink([false, true]).show(ui, |ui| {
        contents(app, ui, &v, &folder);
    });
}

/// The design in front isn't in a folder that keeps versions.
fn not_kept(app: &mut SolveApp, ui: &mut egui::Ui) {
    let path = app.session.path.clone().filter(|p| !p.is_empty());
    match &path {
        Some(p) => {
            ui_kit::text(ui, &format("ui.versions_not_kept", &[("name", &file_name(p))]), ui_kit::Text::Note("versions"));
            let parent = Path::new(p).parent().map(|d| d.to_string_lossy().into_owned()).filter(|d| !d.is_empty());
            if let Some(dir) = parent {
                let label = format("ui.versions_keep_this", &[("folder", &file_name(&dir))]);
                if ui_kit::primary_button(ui, &label).clicked() {
                    act(app, After::Started(dir.clone()), "versions.start", json!({"folder": dir}));
                }
            }
        }
        None => {
            ui_kit::text(ui, tr("ui.versions_unsaved"), ui_kit::Text::Note("versions"));
        }
    }
    if ui_kit::button(ui, tr("ui.versions_keep_other"), ui_kit::Button::Normal).clicked() {
        app.pick_folder(tr("ui.versions_pick_title"), |app, folder| {
            if let Some(folder) = folder {
                act(app, After::Started(folder.clone()), "versions.start", json!({"folder": folder}));
            }
        });
    }
    ui_kit::text(ui, tr("ui.versions_kept_note"), ui_kit::Text::Hint("versions"));
    if !app.preferences.versions_folders.is_empty() {
        ui_kit::text(ui, tr("ui.versions_other_folders"), ui_kit::Text::Hint("versions"));
    }
}

fn contents(app: &mut SolveApp, ui: &mut egui::Ui, v: &View, folder: &str) {
    let t = Tokens::get();
    let folders = app.preferences.versions_folders.clone();
    if app.versions.scope == Scope::Folder && folders.len() > 1 && v.design.is_none() {
        let names: Vec<String> = folders.iter().map(|f| file_name(f)).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut picked = app.versions.picked.min(folders.len() - 1);
        ui_kit::dropdown(ui, "versions_folder", &mut picked, &names);
        app.versions.picked = picked;
    } else {
        ui_kit::text(ui, &file_name(folder), ui_kit::Text::Heading("versions_folder"));
    }
    let status = match &app.versions.status {
        Some(Ok(s)) => s.clone(),
        Some(Err(e)) => {
            ui_kit::text(ui, &format("ui.versions_failed", &[("error", e)]), ui_kit::Text::Error("versions"));
            return;
        }
        None => {
            ui_kit::text(ui, tr("ui.versions_looking"), ui_kit::Text::Hint("versions"));
            return;
        }
    };
    if status["tracked"] != true {
        // Kept before, but its history is gone (another computer, or deleted): start again.
        ui_kit::text(ui, tr("ui.versions_no_history_here"), ui_kit::Text::Note("versions"));
        if ui_kit::primary_button(ui, &format("ui.versions_keep_this", &[("folder", &file_name(folder))])).clicked() {
            act(app, After::Started(folder.to_string()), "versions.start", json!({"folder": folder}));
        }
        return;
    }
    let in_scope = |path: &str| app.versions.scope == Scope::Folder || v.files.contains(path);
    let changes: Vec<(String, String, bool)> = status["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["path"].as_str()?.to_string(), c["change"].as_str().unwrap_or("changed").to_string(), c["local"] != false)))
        .filter(|(p, _, _)| in_scope(p))
        .collect();
    let part_view = app.versions.scope == Scope::Design && !v.assembly;

    // Check In.
    ui_kit::section_count(ui, "versions_check_in", tr("ui.versions_check_in_title"), true, None::<usize>, |ui| {
        let dirty_front = app.session.is_dirty() && v.design.is_some();
        if dirty_front {
            ui_kit::text(ui, tr("ui.versions_unsaved_front"), ui_kit::Text::Note("versions"));
            if ui_kit::button(ui, tr("ui.versions_save"), ui_kit::Button::Normal).clicked() {
                crate::save_job::start(app, "file.save", json!({"overwrite": true}));
                app.versions.soon();
            }
        }
        ui_kit::text_area(ui, tr("ui.versions_message_hint"), egui::Id::unique("sc_versions_message"), &mut app.versions.message, 2, 2000);
        if app.versions.message.is_empty() {
            ui_kit::text(ui, tr("ui.versions_message_hint"), ui_kit::Text::Hint("versions_message"));
        }
        let design_change = v.design.as_deref().filter(|d| part_view && changes.iter().any(|(p, c, _)| p == d && c != "deleted"));
        let revision = design_change.map(|d| next_revision(app.versions.history.as_ref().and_then(|h| h.as_ref().ok()), d));
        if let Some(rev) = &revision {
            ui_kit::checkbox(ui, &format("ui.versions_release", &[("revision", rev)]), &mut app.versions.release);
        }
        let chosen: Vec<String> = changes.iter().filter(|(p, _, _)| !app.versions.left_out.contains(p)).map(|(p, _, _)| p.clone()).collect();
        let key = match chosen.len() {
            0 => "ui.versions_check_in_title",
            1 => "ui.versions_check_in_one",
            _ => "ui.versions_check_in_other",
        };
        let label = format(key, &[("count", &chosen.len().to_string())]);
        let ready = !chosen.is_empty() && !app.versions.message.trim().is_empty() && app.versions.acting.is_none() && app.versions.queued.is_none();
        let clicked = ui.add_enabled_ui(ready, |ui| ui_kit::primary_button(ui, &label)).inner.clicked();
        if clicked {
            let release = match (app.versions.release, design_change, &revision) {
                (true, Some(d), Some(rev)) => Some((d.to_string(), rev.clone())),
                _ => None,
            };
            let mut paths = chosen.clone();
            if release.is_some() {
                // The revision goes into the part's properties (and bills of materials) first.
                let _ = app.run("document.metadata.set", json!({"values": {"revision": revision.clone().unwrap_or_default()}}));
                crate::save_job::start(app, "file.save", json!({"overwrite": true}));
                if let Some(d) = design_change
                    && !paths.iter().any(|p| p == d)
                {
                    paths.push(d.to_string());
                }
            }
            app.versions.queued = Some(Queued { folder: folder.to_string(), message: app.versions.message.trim().to_string(), paths, release });
        }
        let who = if app.preferences.versions_author.trim().is_empty() {
            std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default()
        } else {
            app.preferences.versions_author.trim().to_string()
        };
        ui_kit::text(ui, &format("ui.versions_as", &[("name", &who)]), ui_kit::Text::Hint("versions_as"));
        if ui_kit::button(ui, tr("ui.versions_change_name"), ui_kit::Button::Normal).clicked() {
            app.versions.editing_name = !app.versions.editing_name;
        }
        if app.versions.editing_name {
            ui_kit::hint_field(ui, &mut app.preferences.versions_author, tr("ui.versions_your_name"));
        }
    });

    // My changes.
    ui_kit::section_count(ui, "versions_changes", tr("ui.versions_changes"), true, Some(changes.len()), |ui| {
        if changes.is_empty() {
            let key = if part_view { "ui.versions_no_changes_part" } else { "ui.versions_no_changes" };
            ui_kit::text(ui, tr(key), ui_kit::Text::Hint("versions_changes"));
        }
        for (path, change, local) in &changes {
            let mut on = !app.versions.left_out.contains(path);
            let mut label = std::format!("{} · {}", file_name(path), change_word(change));
            if !local {
                label.push_str(&std::format!(" · {}", tr("ui.versions_cloud")));
            }
            let response = ui_kit::checkbox(ui, &label, &mut on).on_hover_text(path);
            if response.changed() {
                if on {
                    app.versions.left_out.remove(path);
                } else {
                    app.versions.left_out.insert(path.clone());
                }
            }
        }
    });

    // History.
    let title = tr(if part_view { "ui.versions_history_part" } else { "ui.versions_history" }).to_string();
    let history = match &app.versions.history {
        Some(Ok(h)) => h.clone(),
        Some(Err(e)) => {
            ui_kit::text(ui, &format("ui.versions_failed", &[("error", e)]), ui_kit::Text::Error("versions"));
            return;
        }
        None => Value::Null,
    };
    let commits: Vec<Value> = history["commits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            app.versions.scope == Scope::Folder
                || c["files"].as_array().into_iter().flatten().any(|f| f["path"].as_str().is_some_and(|p| v.files.contains(p)))
        })
        .cloned()
        .collect();
    ui_kit::section_count(ui, "versions_history", &title, true, Some(commits.len()), |ui| {
        if commits.is_empty() {
            ui_kit::text(ui, tr("ui.versions_no_history"), ui_kit::Text::Hint("versions_history"));
        }
        // The history as a graph, as a version-control tree shows it: a rail down the left with
        // a node for each version (the newest ringed, a released one in the release colour), its
        // revisions and names as chips, and the changes not checked in yet as a hollow node on top.
        let more = history["more"] == true;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if !changes.is_empty() && !commits.is_empty() {
                let label = std::format!("{} ({})", tr("ui.versions_changes"), changes.len());
                graph_row(ui, None, &GraphRow { title: &label, pending: true, ..Default::default() }, (false, true));
            }
            for (index, c) in commits.iter().enumerate() {
                let id = c["id"].as_str().unwrap_or("").to_string();
                let line = c["message"].as_str().unwrap_or("").lines().next().unwrap_or("");
                let revisions: Vec<String> = c["revisions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|r| app.versions.scope == Scope::Folder || r["path"].as_str().is_some_and(|p| v.files.contains(p)))
                    .filter_map(|r| r["revision"].as_str().map(str::to_string))
                    .collect();
                let names: Vec<String> = c["versions"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(str::to_string)).collect();
                let selected = app.versions.selected.as_deref() == Some(id.as_str());
                let by =
                    format("ui.versions_by", &[("author", c["author"].as_str().unwrap_or("")), ("when", &when(c["time"].as_i64().unwrap_or(0)))]);
                let short = c["short"].as_str().unwrap_or("");
                let last = index + 1 == commits.len() && !more;
                let above = index > 0 || !changes.is_empty();
                let row = GraphRow { title: line, by: &by, short, revisions: &revisions, names: &names, head: index == 0, selected, pending: false };
                if graph_row(ui, Some(&std::format!("version:{short}")), &row, (above, !last)).clicked() {
                    app.versions.selected = if selected { None } else { Some(id.clone()) };
                    app.versions.confirm = None;
                }
                if selected {
                    let under = ui
                        .horizontal(|ui| {
                            ui.add_space(GRAPH_GUTTER);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = t.space_3;
                                ui.add_space(t.space_3);
                                details(app, ui, v, folder, c, part_view);
                                ui.add_space(t.space_4);
                            });
                        })
                        .response
                        .rect;
                    if !last {
                        let x = under.left() + GRAPH_GUTTER / 2.0;
                        ui.painter()
                            .line_segment([egui::pos2(x, under.top()), egui::pos2(x, under.bottom())], egui::Stroke::new(GRAPH_RAIL, t.accent));
                    }
                }
            }
        });
        if more {
            ui_kit::text(ui, tr("ui.versions_more"), ui_kit::Text::Hint("versions_history"));
        }
    });
}

/// Width of the history graph's rail column, and the rail's thickness.
const GRAPH_GUTTER: f32 = 22.0;
const GRAPH_RAIL: f32 = 2.0;

/// One row of the history graph.
#[derive(Default)]
struct GraphRow<'a> {
    title: &'a str,
    by: &'a str,
    short: &'a str,
    revisions: &'a [String],
    names: &'a [String],
    /// The newest version: what the folder holds now.
    head: bool,
    selected: bool,
    /// The changes not checked in yet: a hollow node, not a version.
    pending: bool,
}

/// A row of the history graph: its node on the rail (`rail`: whether the rail runs on above and
/// below it), the version's first line with its revision and name chips, and who checked it in
/// and when underneath. `key` makes it a row to pick; the changes row has none.
fn graph_row(ui: &mut egui::Ui, key: Option<&str>, row: &GraphRow, rail: (bool, bool)) -> egui::Response {
    let t = Tokens::get();
    let two_lines = !row.by.is_empty();
    let height = if two_lines { t.font_small + t.font_caption + t.space_12 + t.space_4 } else { t.font_small + t.space_12 };
    let sense = if key.is_some() { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
    if let Some(key) = key {
        crate::scenario::publish_target(&std::format!("dialog:list:{key}"), rect);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter_at(rect.expand(1.0));
    let body = egui::Rect::from_min_max(egui::pos2(rect.left() + GRAPH_GUTTER, rect.top() + 1.0), egui::pos2(rect.right(), rect.bottom() - 1.0));
    if row.selected {
        painter.rect_filled(body, t.radius_md, t.selected_bg);
    } else if key.is_some() && response.hovered() {
        painter.rect_filled(body, t.radius_md, t.hover);
    }
    // The rail and the node.
    let first_line = rect.top() + t.space_6 + t.font_small / 2.0;
    let node = egui::pos2(rect.left() + GRAPH_GUTTER / 2.0, first_line);
    let rail_color = if row.pending { t.text_dim } else { t.accent };
    if rail.0 {
        painter.line_segment([egui::pos2(node.x, rect.top() - 1.0), node], egui::Stroke::new(GRAPH_RAIL, t.accent));
    }
    if rail.1 {
        painter.line_segment([node, egui::pos2(node.x, rect.bottom() + 1.0)], egui::Stroke::new(GRAPH_RAIL, rail_color));
    }
    let released = !row.revisions.is_empty();
    let color = if released { t.form_success } else { t.accent };
    if row.pending {
        painter.circle_filled(node, 4.5, t.panel);
        painter.circle_stroke(node, 4.5, egui::Stroke::new(1.5, t.text_dim));
    } else if row.head {
        painter.circle_filled(node, 7.0, t.panel);
        painter.circle_stroke(node, 6.5, egui::Stroke::new(1.5, color));
        painter.circle_filled(node, 3.5, color);
    } else {
        painter.circle_filled(node, 5.5, t.panel);
        painter.circle_filled(node, if released { 4.5 } else { 4.0 }, color);
    }
    // Chips from the right: names, then revisions; the first line takes what is left.
    let chip_font = crate::ui_kit::type_face::medium(t.font_micro);
    let mut right = body.right() - t.space_6;
    let chips = row.names.iter().map(|n| (n, t.accent_soft, t.accent)).chain(row.revisions.iter().map(|r| (r, t.form_success_fill, t.form_success)));
    for (text, fill, ink) in chips {
        let galley = painter.layout_no_wrap(text.clone(), chip_font.clone(), ink);
        let size = egui::vec2(galley.size().x + t.space_8, t.font_micro + t.space_6);
        if right - size.x < body.left() + body.width() * 0.4 {
            break;
        }
        let chip = egui::Rect::from_center_size(egui::pos2(right - size.x / 2.0, first_line), size);
        painter.rect_filled(chip, size.y / 2.0, fill);
        painter.galley(chip.center() - galley.size() / 2.0, galley, ink);
        right = chip.left() - t.space_4;
    }
    let one_line = |text: &str, font: egui::FontId, color: egui::Color32, width: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
        job.wrap = egui::text::TextWrapping { max_width: width.max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        painter.layout_job(job)
    };
    let left = body.left() + t.space_6;
    let ink = if row.selected {
        t.selected_ink
    } else if row.pending {
        t.text_dim
    } else {
        t.text
    };
    let font = if row.head || row.selected { crate::ui_kit::type_face::semibold(t.font_small) } else { egui::FontId::proportional(t.font_small) };
    let title = one_line(row.title, font, ink, right - left);
    painter.galley(egui::pos2(left, first_line - title.size().y / 2.0), title, ink);
    if two_lines {
        let second = first_line + t.font_small / 2.0 + t.space_4 + t.font_caption / 2.0;
        let dim = if row.selected { t.selected_ink } else { t.text_dim };
        let id = painter.layout_no_wrap(row.short.to_owned(), egui::FontId::monospace(t.font_micro), dim);
        let id_left = body.right() - t.space_6 - id.size().x;
        let by = one_line(row.by, egui::FontId::proportional(t.font_caption), dim, id_left - t.space_6 - left);
        painter.galley(egui::pos2(left, second - by.size().y / 2.0), by, dim);
        painter.galley(egui::pos2(id_left, second - id.size().y / 2.0), id, dim);
    }
    response.on_hover_text(row.title)
}

/// The picked version: its files and what can be done with it.
fn details(app: &mut SolveApp, ui: &mut egui::Ui, v: &View, folder: &str, c: &Value, part_view: bool) {
    let t = Tokens::get();
    let id = c["id"].as_str().unwrap_or("").to_string();
    let files: Vec<(String, String)> = c["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| Some((f["path"].as_str()?.to_string(), f["change"].as_str().unwrap_or("").to_string())))
        .collect();
    ui.indent("versions_details", |ui| {
        let message = c["message"].as_str().unwrap_or("");
        if message.lines().count() > 1 {
            ui_kit::text(ui, message, ui_kit::Text::Note("versions_message"));
        }
        for (path, change) in &files {
            ui_kit::text(ui, &std::format!("{} · {}", file_name(path), change_word(change)), ui_kit::Text::Note("versions_file"));
        }
        ui.add_space(t.space_2);
        // The part these actions are about: the design in front, else the version's first file.
        let part = v
            .design
            .clone()
            .filter(|d| files.iter().any(|(p, ch)| p == d && ch != "deleted") || !part_view)
            .filter(|d| files.iter().any(|(p, _)| p == d))
            .or_else(|| files.iter().find(|(_, ch)| ch != "deleted").map(|(p, _)| p.clone()));
        let confirm = app.versions.confirm.clone();
        match confirm {
            Some(Confirm::RestorePart(p)) => {
                let name = file_name(&p);
                let front_dirty = app.session.is_dirty() && v.design.as_deref() == Some(p.as_str());
                if front_dirty {
                    ui_kit::text(ui, &format("ui.versions_restore_dirty", &[("name", &name)]), ui_kit::Text::Error("versions"));
                } else {
                    ui_kit::text(ui, &format("ui.versions_restore_part_confirm", &[("name", &name)]), ui_kit::Text::Note("versions"));
                }
                if yes_no(app, ui, "ui.versions_restore", !front_dirty) {
                    act(app, After::Restored, "versions.restore", json!({"folder": folder, "commit": id, "path": p}));
                }
            }
            Some(Confirm::RestoreAll) => {
                ui_kit::text(ui, tr("ui.versions_restore_all_confirm"), ui_kit::Text::Note("versions"));
                if yes_no(app, ui, "ui.versions_restore", true) {
                    act(app, After::Restored, "versions.restore", json!({"folder": folder, "commit": id}));
                }
            }
            Some(Confirm::Name) => {
                ui_kit::hint_field(ui, &mut app.versions.name, tr("ui.versions_name_hint"));
                let ready = !app.versions.name.trim().is_empty();
                if yes_no(app, ui, "ui.versions_name", ready) {
                    let mut params = json!({"folder": folder, "commit": id, "version": app.versions.name.trim()});
                    if !app.preferences.versions_author.trim().is_empty() {
                        params["author"] = json!(app.preferences.versions_author.trim());
                    }
                    act(app, After::Named, "versions.name", params);
                }
            }
            Some(Confirm::Release(p)) => {
                let name = file_name(&p);
                ui_kit::text(ui, &format("ui.versions_release_confirm", &[("name", &name)]), ui_kit::Text::Note("versions"));
                ui_kit::hint_field(ui, &mut app.versions.name, "Rev B");
                let ready = !app.versions.name.trim().is_empty();
                if yes_no(app, ui, "ui.versions_release_do", ready) {
                    let revision = app.versions.name.trim().to_string();
                    let after = After::Released { name, revision: revision.clone() };
                    act(app, after, "versions.name", json!({"folder": folder, "commit": id, "path": p, "revision": revision}));
                }
            }
            Some(Confirm::NewPart(p)) => {
                ui_kit::hint_field(ui, &mut app.versions.name, tr("ui.versions_new_part_hint"));
                let name = app.versions.name.trim().to_string();
                let ext = Path::new(&p).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                let to = match p.rsplit_once('/') {
                    Some((dir, _)) => std::format!("{dir}/{name}.{ext}"),
                    None => std::format!("{name}.{ext}"),
                };
                let ready = !name.is_empty() && !name.contains(['/', '\\']) && !Path::new(folder).join(&to).exists();
                if yes_no(app, ui, "ui.versions_save_new", ready) {
                    act(app, After::NewPart, "versions.copy", json!({"folder": folder, "commit": id, "path": p, "to": to}));
                }
            }
            None => {
                if let Some(p) = &part {
                    let name = file_name(p);
                    if ui_kit::button(ui, tr("ui.versions_open_version"), ui_kit::Button::Normal).clicked() {
                        act(app, After::Open, "versions.copy", json!({"folder": folder, "commit": id, "path": p}));
                    }
                    if ui_kit::button(ui, format("ui.versions_restore_part", &[("name", &name)]), ui_kit::Button::Normal).clicked() {
                        app.versions.confirm = Some(Confirm::RestorePart(p.clone()));
                    }
                    let released = c["revisions"].as_array().into_iter().flatten().any(|r| r["path"] == p.as_str());
                    if part_view && !released {
                        let rev = next_revision(app.versions.history.as_ref().and_then(|h| h.as_ref().ok()), p);
                        if ui_kit::button(ui, format("ui.versions_release_rev", &[("name", &name), ("revision", &rev)]), ui_kit::Button::Normal)
                            .clicked()
                        {
                            app.versions.name = rev;
                            app.versions.confirm = Some(Confirm::Release(p.clone()));
                        }
                    }
                    if solidcraft_engine::io::is_design_path(p)
                        && ui_kit::button(ui, tr("ui.versions_save_new_more"), ui_kit::Button::Normal).clicked()
                    {
                        app.versions.name = format("ui.versions_copy_name", &[("name", &name)]);
                        app.versions.confirm = Some(Confirm::NewPart(p.clone()));
                    }
                }
                if !part_view {
                    if ui_kit::button(ui, tr("ui.versions_restore_all"), ui_kit::Button::Normal).clicked() {
                        app.versions.confirm = Some(Confirm::RestoreAll);
                    }
                    if ui_kit::button(ui, tr("ui.versions_name_version"), ui_kit::Button::Normal).clicked() {
                        app.versions.name.clear();
                        app.versions.confirm = Some(Confirm::Name);
                    }
                }
            }
        }
    });
}

/// The confirm's action and Cancel. True when the action was pressed (the question closes).
fn yes_no(app: &mut SolveApp, ui: &mut egui::Ui, action: &str, enabled: bool) -> bool {
    let mut yes = false;
    ui.horizontal(|ui| {
        yes = ui.add_enabled_ui(enabled, |ui| ui_kit::primary_button(ui, tr(action))).inner.clicked();
        if ui_kit::button(ui, tr("ui.cancel"), ui_kit::Button::Normal).clicked() {
            app.versions.confirm = None;
        }
    });
    if yes {
        app.versions.confirm = None;
    }
    yes
}

/// The Browser panel header's switch between the Browser and the Versions tab, a small button
/// ending at `right`; returns where the header's next item ends.
pub fn header_switch(app: &mut SolveApp, ui: &mut egui::Ui, rect: egui::Rect, right: f32) -> f32 {
    let t = Tokens::get();
    let size = t.control_height.min(rect.height());
    let r = egui::Rect::from_center_size(egui::pos2(right - size / 2.0, rect.center().y), egui::vec2(size, size));
    let label = tr(if app.versions.open { "ui.versions_show_browser" } else { "ui.versions_show" });
    let response = ui_kit::quiet_icon_button_at(ui, r, "restore", label);
    crate::scenario::publish_target("browser:versions", r);
    if response.on_hover_text(label).clicked() {
        app.versions.open = !app.versions.open;
        app.versions.soon();
    }
    r.left() - t.space_4
}

#[cfg(test)]
#[path = "versions_tests.rs"]
mod tests;
