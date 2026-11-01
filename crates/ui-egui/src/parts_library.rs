//! The Parts Library window (docs/parts-library.md): the parts in the chosen library folder
//! (`library.parts.search`'s list), searched as you type, and Insert, which runs
//! `component.insert_design` for a design (a linked component) or `file.insert_step` for a STEP
//! or IGES file (copied in). The folder is a preference, so the window opens on it next time. The
//! toolbar's Parts Library (`ui.start` of `library.parts.search`) opens it.

use egui::RichText;
use serde_json::json;
use solidcraft_engine::io::parts_library::{self, Entry, Kind, Library};

use crate::SolveApp;
use crate::i18n::{format, tr};
use crate::theme::Tokens;
use crate::ui_kit::Column;

/// Rows drawn at once; the count says how many more matched.
const MAX_ROWS: usize = 500;

/// A scan running beside the window.
#[cfg(not(target_arch = "wasm32"))]
type Scanning = std::sync::mpsc::Receiver<Result<Library, String>>;
#[cfg(target_arch = "wasm32")]
type Scanning = ();

/// The window's state.
#[derive(Default)]
pub struct PartsLibrary {
    /// What the search box holds.
    pub query: String,
    /// The library as last read, for the folder it was read from.
    pub library: Option<Library>,
    /// The picked part's path in the library.
    pub picked: Option<String>,
    /// A scan of the folder, while it runs.
    scanning: Option<Scanning>,
    /// The last refusal, or what the last Insert did.
    pub note: Option<(String, bool)>,
}

impl PartsLibrary {
    /// Read `folder` again, beside the window.
    fn rescan(&mut self, folder: &str) {
        self.picked = None;
        self.note = None;
        if folder.is_empty() {
            self.library = None;
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let folder = folder.to_string();
            std::thread::spawn(move || {
                let _ = tx.send(parts_library::scan(&folder).map_err(|e| e.to_string()));
            });
            self.scanning = Some(rx);
        }
        #[cfg(target_arch = "wasm32")]
        self.finish(parts_library::scan(folder).map_err(|e| e.to_string()));
    }

    fn finish(&mut self, result: Result<Library, String>) {
        self.scanning = None;
        match result {
            Ok(library) => self.library = Some(library),
            Err(e) => {
                self.library = None;
                self.note = Some((e, true));
            }
        }
    }

    /// Take a finished scan's result.
    fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(rx) = &self.scanning {
            match rx.try_recv() {
                Ok(result) => self.finish(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.finish(Err(tr("ui.parts_library_none").to_string())),
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }
}

/// Open the window on the library folder chosen last time.
pub fn open(app: &mut SolveApp) {
    let mut w = PartsLibrary::default();
    w.rescan(&app.preferences.parts_library_folder);
    app.parts_library = Some(w);
}

/// Make `folder` the library and read it.
pub fn choose(app: &mut SolveApp, folder: String) {
    app.preferences.parts_library_folder = folder;
    if let Some(w) = app.parts_library.as_mut() {
        w.rescan(&app.preferences.parts_library_folder);
    }
}

fn kind_label(entry: &Entry) -> String {
    match entry.kind {
        Kind::Design => "SolidCraft".into(),
        Kind::Cad => entry.format.to_uppercase(),
    }
}

/// Put `entry` into the design, as the toolbar's Insert Design does: a design as a linked
/// component (a library on a server is the user's choice, like a file picked in a dialog), a CAD
/// file copied in beside the window, which reports how it went. `Some`: what to say now.
fn insert(app: &mut SolveApp, library: &Library, entry: &Entry) -> Option<(String, bool)> {
    let file = library.absolute(entry).to_string_lossy().into_owned();
    match entry.kind {
        Kind::Design => {
            let hosts: Vec<String> = solidcraft_engine::io::vfs::network_host(&file).into_iter().collect();
            let inserted = solidcraft_engine::io::vfs::allowing_hosts(&hosts, || app.run("component.insert_design", json!({"path": file})));
            Some(match inserted {
                Ok(_) => {
                    app.fit_view();
                    (format("ui.parts_library_inserted", &[("name", &entry.name)]), false)
                }
                Err(e) => (e, true),
            })
        }
        Kind::Cad => {
            app.insert_path_async(&file);
            None
        }
    }
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut w) = app.parts_library.take() else { return };
    w.poll(ctx);
    let t = Tokens::get();
    let folder = app.preferences.parts_library_folder.clone();
    let (mut choose_folder, mut refresh, mut insert_picked) = (false, false, false);
    let closed =
        crate::ui_kit::Window::new(egui::Id::new("sc_parts_library"), "folder", tr("cmd.library.parts.search")).cards(3.0).show_body(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if crate::ui_kit::button(ui, tr("ui.parts_library_choose"), crate::ui_kit::Button::Normal).clicked() {
                    choose_folder = true;
                }
                if !folder.is_empty() && crate::ui_kit::button(ui, tr("ui.parts_library_refresh"), crate::ui_kit::Button::Normal).clicked() {
                    refresh = true;
                }
                ui.label(RichText::new(&folder).small().color(t.text_dim));
            });
            if folder.is_empty() {
                ui.label(tr("ui.parts_library_none"));
                return;
            }
            crate::ui_kit::search_field(ui, egui::Id::new("sc_parts_library_search"), &mut w.query, tr("ui.parts_library_search"));
            if w.scanning.is_some() {
                ui.label(RichText::new(tr("ui.parts_library_reading")).color(t.text_dim));
                return;
            }
            let Some(library) = &w.library else { return };
            let (found, matched) = parts_library::search(library, &w.query, MAX_ROWS);
            // Insert Part and the count above the list, so a long list never hides them.
            let entry = w.picked.as_deref().and_then(|p| library.entries.iter().find(|e| e.path == p));
            ui.horizontal_wrapped(|ui| {
                let button = ui
                    .add_enabled_ui(entry.is_some(), |ui| crate::ui_kit::button(ui, tr("ui.parts_library_insert"), crate::ui_kit::Button::Normal))
                    .inner;
                if button.clicked() {
                    insert_picked = true;
                }
                let shown = found.len().min(matched).to_string();
                let total = library.entries.len().to_string();
                ui.label(RichText::new(format("ui.parts_library_count", &[("shown", &shown), ("total", &total)])).color(t.text_dim));
            });
            if let Some(entry) = entry {
                let hint = match entry.kind {
                    Kind::Design => "ui.parts_library_design_hint",
                    Kind::Cad => "ui.parts_library_cad_hint",
                };
                ui.label(RichText::new(tr(hint)).small().color(t.text_dim));
            }
            if library.truncated {
                ui.label(RichText::new(tr("ui.parts_library_truncated")).small().color(t.text_dim));
            }
            let columns = [
                Column::new(tr("ui.parts_library_col_name"), 1.5).flex(),
                Column::new(tr("ui.parts_library_col_number"), 0.8).flex(),
                Column::new(tr("ui.parts_library_col_folder"), 1.8).flex(),
                Column::new(tr("ui.parts_library_col_type"), 0.6),
            ];
            egui::ScrollArea::vertical().id_salt("sc_parts_library_rows").max_height(crate::ui_kit::table_row_height() * 12.0).show(ui, |ui| {
                crate::ui_kit::table_custom(ui, "parts_library", &columns, tr("ui.parts_library_no_match"), |ui, table| {
                    for entry in &found {
                        let picked = w.picked.as_deref() == Some(entry.path.as_str());
                        // Plain text, not selectable: a click anywhere on the row picks the part.
                        let cell = |ui: &mut egui::Ui, text: RichText| ui.add(egui::Label::new(text).selectable(false).truncate());
                        let row = table.row(ui, &entry.path, picked, |row| {
                            row.cell(|ui| cell(ui, RichText::new(&entry.name)));
                            row.cell(|ui| cell(ui, RichText::new(entry.part_number.as_deref().unwrap_or(""))));
                            row.cell(|ui| cell(ui, RichText::new(entry.folder.replace('/', " › ")).color(t.text_dim)));
                            row.cell(|ui| cell(ui, RichText::new(kind_label(entry)).color(t.text_dim)));
                        });
                        if row.clicked() {
                            w.picked = Some(entry.path.clone());
                        }
                        if row.double_clicked() {
                            w.picked = Some(entry.path.clone());
                            insert_picked = true;
                        }
                        row.on_hover_text(&entry.path);
                    }
                });
            });
            if let Some((note, error)) = &w.note {
                ui.label(RichText::new(note).color(if *error { t.error } else { t.text_dim }));
            }
        });
    if refresh {
        w.rescan(&folder);
    }
    if insert_picked
        && let Some(library) = w.library.clone()
        && let Some(entry) = w.picked.as_deref().and_then(|p| library.entries.iter().find(|e| e.path == p))
    {
        w.note = insert(app, &library, entry);
    }
    if !closed {
        app.parts_library = Some(w);
    }
    if choose_folder {
        app.pick_folder(tr("ui.parts_library_choose_title"), |app, folder| {
            if let Some(folder) = folder {
                choose(app, folder);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use serde_json::json;

    use crate::scenario::Harness;

    /// A library folder of `files` under the harness's folder (written as tiny placeholders, or
    /// copied from the given source).
    fn library(h: &Harness, files: &[(&str, Option<&std::path::Path>)]) -> String {
        let root = h.dir.join("LIBRARY");
        for (f, from) in files {
            let path = root.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            match from {
                Some(from) => std::fs::copy(from, &path).map(|_| ()).unwrap(),
                None => std::fs::write(&path, b"x").unwrap(),
            }
        }
        root.to_string_lossy().into_owned()
    }

    fn wait_for_scan(h: &mut Harness) {
        for _ in 0..200 {
            h.frames(1);
            if h.app.parts_library.as_ref().is_some_and(|w| w.scanning.is_none()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the scan never finished");
    }

    #[test]
    fn the_window_asks_for_a_folder_then_lists_and_searches_its_parts() {
        let mut h = Harness::new();
        let folder =
            library(&h, &[("THRUSTERS/BLUE ROBOTICS/T200/T200.STEP", None), ("CAMERAS/Low-light camera.igs", None), ("CAMERAS/notes.pdf", None)]);
        h.step(&json!({"start": "library.parts.search"})).unwrap();
        assert!(h.app.parts_library.is_some(), "ui.start opens the window");
        h.frames(2);
        assert!(h.widget("Choose Folder…").is_some());
        assert!(h.app.parts_library.as_ref().unwrap().library.is_none(), "no folder chosen yet");
        super::choose(&mut h.app, folder.clone());
        wait_for_scan(&mut h);
        assert_eq!(h.app.preferences.parts_library_folder, folder, "the folder is remembered");
        h.frames(2);
        assert!(h.widget("2 of 2 parts").is_some());
        assert!(h.widget("T200").is_some() && h.widget("Low-light camera").is_some());
        h.app.parts_library.as_mut().unwrap().query = "blue robotics".into();
        h.frames(2);
        assert!(h.widget("1 of 2 parts").is_some(), "the vendor's folder is searched");
        assert!(h.widget("Low-light camera").is_none());
        h.app.parts_library.as_mut().unwrap().query = "sonar".into();
        h.frames(2);
        assert!(h.widget("No parts match your search.").is_some());
        let undo = h.app.session.undo.len();
        assert_eq!(h.app.session.undo.len(), undo, "searching changes nothing");
    }

    #[test]
    fn insert_links_a_library_design_and_reopening_keeps_the_folder() {
        let mut h = Harness::new();
        h.app.run("solid.box", json!({"length": 40, "width": 20, "height": 3})).unwrap();
        h.app.run("document.metadata.set", json!({"values": {"part_number": "PL-100", "description": "Mounting plate"}})).unwrap();
        let part = h.dir.join("plate.solidcraft");
        h.app.run("file.save", json!({"path": part.to_string_lossy()})).unwrap();
        crate::documents::new_design(&mut h.app);
        let folder = library(&h, &[("MOUNTING HARDWARE/Plate.solidcraft", Some(&part))]);
        h.app.preferences.parts_library_folder = folder;
        h.step(&json!({"start": "library.parts.search"})).unwrap();
        wait_for_scan(&mut h);
        h.frames(2);
        assert!(h.widget("PL-100").is_some(), "the design's part number shows");
        assert!(h.widget("SolidCraft").is_some());
        h.step(&json!({"widget": "Plate"})).unwrap();
        h.frames(2);
        let undo = h.app.session.undo.len();
        h.step(&json!({"widget": "Insert Part"})).unwrap();
        h.frames(2);
        assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
        assert_eq!(h.app.session.doc.links.len(), 1, "inserted as a link");
        let note = h.app.parts_library.as_ref().unwrap().note.clone().unwrap();
        assert_eq!(note, ("Inserted Plate.".to_string(), false));
    }
}
