//! Changed linked parts (docs/parts-library.md, "Changed parts"). While a design with linked
//! parts is open, the window looks at their files every couple of seconds, by size and date,
//! which costs next to nothing. When one changed, or another design comes to the front,
//! `component.links` compares each file's content with what its link last read (`changed`),
//! beside the window. A changed project part comes in at once (`component.update_links` for
//! it, beside the window, one undo step) while the preference "Update changed linked parts
//! automatically" is on; a changed library part (one in a Parts Library folder) stays as it is
//! until the user presses Update, so a vendor's new revision never changes a design unasked.

use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::Receiver;

use serde_json::{Value, json};

use crate::SolveApp;
use crate::i18n::{format, tr};

/// How often the linked files are looked at, in seconds.
const EVERY: f64 = 2.0;

/// A linked part whose file changed since its link last read it.
#[derive(Clone, Debug, PartialEq)]
pub struct Changed {
    pub component: u64,
    pub name: String,
    pub file: String,
    /// The file's digest now: putting it off holds until the file changes again.
    pub digest: String,
}

/// The watch over the front design's linked parts.
#[derive(Default)]
pub struct Watch {
    /// The design the sizes and dates below were seen for.
    design: Option<(Option<String>, usize)>,
    /// Each linked file's size and modification time, as last seen.
    seen: HashMap<String, (u64, Option<std::time::SystemTime>)>,
    next: f64,
    checking: Option<Receiver<Vec<Changed>>>,
    /// Changed library parts offered with Update and Not Now.
    pub notice: Vec<Changed>,
    /// Changed project parts waiting for the window to be free to bring them in.
    pending: Vec<Changed>,
    /// Parts put off with Not Now, by component and the file's digest then.
    dismissed: BTreeSet<(u64, String)>,
}

impl Watch {
    /// Look again at the next frame (tests, and after the design's links change).
    pub fn soon(&mut self) {
        self.next = 0.0;
    }
}

/// The links of the front design: component, name and file, for the files that are found.
fn links(app: &mut SolveApp) -> Vec<(u64, String, String)> {
    let Ok(r) = app.session.execute("component.links", &json!({})) else { return Vec::new() };
    r["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["found"] == true)
        .filter_map(|l| Some((l["component"].as_u64()?, l["name"].as_str().unwrap_or("").to_string(), l["file"].as_str()?.to_string())))
        .collect()
}

/// A file inside one of the user's Parts Library folders: a library part, which stays pinned.
fn in_library(app: &SolveApp, file: &str) -> bool {
    let file = std::path::Path::new(file);
    app.preferences.parts_libraries.iter().any(|l| !l.folder.is_empty() && file.starts_with(&l.folder))
}

/// The front design's linked components whose files are in a Parts Library folder: library
/// parts, which change only when the user presses Update.
pub(crate) fn library_components(app: &mut SolveApp) -> Vec<u64> {
    if app.preferences.parts_libraries.is_empty() {
        return Vec::new();
    }
    links(app).into_iter().filter(|(_, _, file)| in_library(app, file)).map(|(c, _, _)| c).collect()
}

/// The window is free for a part to come in: no dialog, sketch, tool, picker or file operation.
fn free(app: &SolveApp) -> bool {
    app.dialog.is_none() && app.session.active_sketch.is_none() && app.tool.is_none() && !app.file_picker_busy() && !app.file_load.busy()
}

/// Bring in the new shapes of `parts`, beside the window, in one undo step.
fn update(app: &mut SolveApp, parts: &[Changed]) {
    if parts.is_empty() {
        return;
    }
    let names = parts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
    let mut request = crate::file_load::Request::new("", crate::file_load::Kind::UpdateLinks);
    request.options.insert("components".into(), json!(parts.iter().map(|c| c.component).collect::<Vec<_>>()));
    if crate::file_load::begin_request(app, request).is_ok() {
        app.set_status(format("ui.linked_updating", &[("names", &names)]), false);
    }
}

/// Once a frame: look at the linked files when it is time, take a finished check, and bring in
/// changed project parts when the window is free.
pub fn tick(app: &mut SolveApp, ctx: &egui::Context) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let now = ctx.input(|i| i.time);
    if let Some(rx) = &app.link_watch.checking {
        match rx.try_recv() {
            Ok(changed) => {
                app.link_watch.checking = None;
                take(app, changed);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => app.link_watch.checking = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
        }
    }
    if !app.link_watch.pending.is_empty() && free(app) {
        let parts = std::mem::take(&mut app.link_watch.pending);
        update(app, &parts);
    }
    if app.session.doc.links.is_empty() {
        app.link_watch.design = None;
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(EVERY));
    if now < app.link_watch.next || app.link_watch.checking.is_some() || app.file_load.busy() {
        return;
    }
    app.link_watch.next = now + EVERY;
    let design = Some((app.session.path.clone(), app.session.doc.links.len()));
    let files = links(app);
    let mut moved = app.link_watch.design != design;
    for (_, _, file) in &files {
        let meta = std::fs::metadata(file).ok();
        let seen = meta.map(|m| (m.len(), m.modified().ok()));
        if let Some(seen) = seen
            && app.link_watch.seen.insert(file.clone(), seen) != Some(seen)
        {
            moved = true;
        }
    }
    app.link_watch.design = design;
    if moved {
        check(app);
    }
}

/// Compare every linked file with what its link last read, beside the window.
fn check(app: &mut SolveApp) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut scratch = app.session.scratch();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("linked-parts-check".into()).spawn(move || {
            let changed = scratch.execute("component.links", &json!({})).map(|r| changed(&r)).unwrap_or_default();
            let _ = tx.send(changed);
        });
        if spawned.is_ok() {
            app.link_watch.checking = Some(rx);
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = app;
}

/// The links `component.links` reports changed, each with its file's digest now.
fn changed(r: &Value) -> Vec<Changed> {
    r["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["changed"] == true)
        .filter_map(|l| {
            Some(Changed {
                component: l["component"].as_u64()?,
                name: l["name"].as_str().unwrap_or("").to_string(),
                file: l["file"].as_str()?.to_string(),
                digest: solidcraft_engine::doc::DesignLink::file_digest(&std::fs::read(l["file"].as_str()?).ok()?),
            })
        })
        .collect()
}

/// Sort a check's changed parts: project parts come in (or wait for the window), library parts
/// and every part with automatic updates off are offered; parts put off stay put off.
fn take(app: &mut SolveApp, changed: Vec<Changed>) {
    let w = &app.link_watch;
    let fresh: Vec<Changed> = changed.into_iter().filter(|c| !w.dismissed.contains(&(c.component, c.digest.clone()))).collect();
    for c in fresh {
        let offer = !app.preferences.linked_parts_auto_update || in_library(app, &c.file);
        let list = if offer { &mut app.link_watch.notice } else { &mut app.link_watch.pending };
        list.retain(|x| x.component != c.component);
        list.push(c);
    }
}

/// The Update notice for changed library parts, when there are any. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    if app.link_watch.notice.is_empty() {
        return;
    }
    // Parts no longer in the design (deleted, or another design in front) are dropped.
    let here: BTreeSet<u64> = app.session.doc.links.keys().copied().collect();
    app.link_watch.notice.retain(|c| here.contains(&c.component));
    if app.link_watch.notice.is_empty() {
        return;
    }
    let t = crate::theme::Tokens::get();
    let names: Vec<String> = app.link_watch.notice.iter().map(|c| c.name.clone()).collect();
    let (mut update_now, mut later) = (false, false);
    let closed = crate::ui_kit::Window::new(egui::Id::new("sc_linked_update"), "update", tr("ui.linked_available_title"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-t.space_16, t.space_24 * 4.0))
        .show_body(ctx, |ui| {
            ui.label(tr("ui.linked_available"));
            for name in &names {
                ui.label(egui::RichText::new(format!("• {name}")).color(t.text_dim));
            }
            ui.horizontal(|ui| {
                update_now = crate::ui_kit::button(ui, tr("ui.linked_update"), crate::ui_kit::Button::Normal).clicked();
                later = crate::ui_kit::button(ui, tr("ui.linked_not_now"), crate::ui_kit::Button::Normal).clicked();
            });
        });
    if update_now {
        let parts = std::mem::take(&mut app.link_watch.notice);
        update(app, &parts);
    } else if later || closed {
        let parts = std::mem::take(&mut app.link_watch.notice);
        app.link_watch.dismissed.extend(parts.into_iter().map(|c| (c.component, c.digest)));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use serde_json::json;

    use crate::scenario::Harness;

    fn save_box(path: &std::path::Path, height: f64) {
        let mut part = solidcraft_engine::Session::default();
        part.execute("solid.box", &json!({"length": 10, "width": 20, "height": height, "body_name": "Plate"})).unwrap();
        part.execute("file.save", &json!({"path": path})).unwrap();
    }

    fn settle(h: &mut Harness, done: impl Fn(&crate::SolveApp) -> bool) {
        for _ in 0..600 {
            h.app.link_watch.soon();
            h.frames(1);
            crate::file_load::tick(&mut h.app, &h.ctx.clone());
            if done(&h.app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("never settled");
    }

    fn source(h: &Harness, name: &str) -> Option<String> {
        let comp = h.app.session.doc.components.iter().find(|c| c.name == name).unwrap().id;
        h.app.session.doc.links[&comp].file_sha256.clone()
    }

    #[test]
    fn a_changed_project_part_comes_in_and_a_changed_library_part_waits_for_update() {
        let mut h = Harness::new();
        let project = h.dir.join("PROJECTS");
        let library = h.dir.join("LIBRARIES");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        let (plate, plug) = (project.join("plate.solidcraft"), library.join("plug.solidcraft"));
        save_box(&plate, 2.0);
        save_box(&plug, 1.0);
        h.app.preferences.parts_libraries =
            vec![crate::parts_library::LibraryRef { name: "Library".into(), folder: library.to_string_lossy().into_owned() }];
        h.app.run("component.insert_design", json!({"path": plate})).unwrap();
        h.app.run("component.insert_design", json!({"path": plug, "translate": [40, 0, 0]})).unwrap();
        settle(&mut h, |app| app.link_watch.checking.is_none() && app.link_watch.design.is_some());
        let (plate_was, plug_was) = (source(&h, "plate"), source(&h, "plug"));
        save_box(&plate, 5.0);
        save_box(&plug, 4.0);
        settle(&mut h, |app| !app.link_watch.notice.is_empty() && app.link_watch.pending.is_empty() && !app.file_load.busy());
        assert_ne!(source(&h, "plate"), plate_was, "the project part came in by itself");
        assert_eq!(source(&h, "plug"), plug_was, "the library part waits");
        assert_eq!(h.app.link_watch.notice.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["plug"]);
        h.frames(2);
        assert!(h.widget("Update available").is_some());
        h.step(&json!({"widget": "Update"})).unwrap();
        settle(&mut h, |app| !app.file_load.busy() && app.link_watch.notice.is_empty());
        assert_ne!(source(&h, "plug"), plug_was, "Update brings it in");
        // With automatic updates off, a project part is offered too, and Not Now puts it off.
        h.app.preferences.linked_parts_auto_update = false;
        save_box(&plate, 7.0);
        settle(&mut h, |app| !app.link_watch.notice.is_empty());
        let offered = source(&h, "plate");
        h.frames(2);
        h.step(&json!({"widget": "Not Now"})).unwrap();
        h.frames(2);
        assert!(h.app.link_watch.notice.is_empty());
        assert_eq!(source(&h, "plate"), offered, "Not Now leaves it as it was");
    }
}
