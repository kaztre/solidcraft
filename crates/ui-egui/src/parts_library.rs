//! The Parts Library window (docs/parts-library.md): the parts in the user's library folders,
//! each library a branch of the category tree on the left (its folders the categories), shown as
//! pictures or a list and searched as you type. Insert Part links a SolidCraft part with
//! `component.insert_design`; a STEP or IGES part is first saved as a SolidCraft part beside it
//! (`library.parts.convert`, beside the window), then linked, so the library lists the design from
//! then on. Convert to SolidCraft Parts does that for every CAD part in the picked category.
//! The libraries are a preference. The toolbar's Library (`ui.start` of `library.parts.search`)
//! opens the window.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use egui::{RichText, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solidcraft_engine::io::parts_library::{self, Entry, Kind, Library};

use crate::SolveApp;
use crate::i18n::{format, tr};
use crate::theme::Tokens;
use crate::ui_kit::Column;

/// Parts drawn at once; the count says how many more matched.
const MAX_ROWS: usize = 500;
/// The largest CAD file whose picture is made without being asked for (a click on it makes a
/// larger one's).
const AUTO_PICTURE_BYTES: u64 = 30 << 20;
/// A picture card's width, in points; its picture keeps the saved thumbnails' proportions.
const CARD_WIDTH: f32 = 132.0;
/// The category tree's width.
const TREE_WIDTH: f32 = 230.0;

/// One library in the list: its name and folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryRef {
    pub name: String,
    pub folder: String,
}

/// The threads that read or draw for the window, and whether they should stop. The app owns
/// them and waits for them as it goes (a picture still on the GPU as the process exits can crash
/// the driver).
#[derive(Default)]
pub struct Workers {
    stop: std::sync::Arc<AtomicBool>,
    #[cfg(not(target_arch = "wasm32"))]
    handles: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl Workers {
    /// Run `work` beside the window, with the flag that asks it to stop; `false` when no thread
    /// could start (the browser).
    fn spawn(&self, name: &str, work: impl FnOnce(std::sync::Arc<AtomicBool>) + Send + 'static) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let stop = self.stop.clone();
            let Ok(handle) = std::thread::Builder::new().name(name.into()).spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(stop)));
            }) else {
                return false;
            };
            if let Ok(mut handles) = self.handles.lock() {
                handles.retain(|h| !h.is_finished());
                handles.push(handle);
            }
            true
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (name, work);
            false
        }
    }

    /// Ask the threads to stop and wait up to `within` for them.
    pub(crate) fn finish(&self, within: std::time::Duration) {
        self.stop.store(true, Ordering::Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let end = std::time::Instant::now() + within;
            while self.handles.lock().is_ok_and(|h| h.iter().any(|h| !h.is_finished())) && std::time::Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = within;
    }
}

/// A library as the window holds it.
struct Loaded {
    lib: LibraryRef,
    library: Option<Library>,
    scanning: Option<Receiver<Result<Library, String>>>,
    error: Option<String>,
}

impl Loaded {
    fn new(lib: LibraryRef, workers: &Workers) -> Self {
        let mut l = Self { lib, library: None, scanning: None, error: None };
        l.rescan(workers);
        l
    }

    fn rescan(&mut self, workers: &Workers) {
        let (tx, rx) = std::sync::mpsc::channel();
        let folder = self.lib.folder.clone();
        if workers.spawn("parts-library-scan", move |_| {
            let _ = tx.send(parts_library::scan(&folder).map_err(|e| e.to_string()));
        }) {
            self.scanning = Some(rx);
        } else {
            self.library = parts_library::scan(&self.lib.folder).map_err(|e| self.error = Some(e.to_string())).ok();
        }
    }

    fn poll(&mut self) -> bool {
        let Some(rx) = &self.scanning else { return false };
        match rx.try_recv() {
            Ok(result) => {
                self.scanning = None;
                match result {
                    Ok(library) => {
                        self.library = Some(library);
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.scanning = None;
                false
            }
        }
    }
}

/// A picture to make: the part's file, and whether it was asked for (a click), which makes one
/// for a large file or one kept only in the cloud too.
struct PictureRequest {
    file: String,
    kind: Kind,
    bytes: u64,
    local: bool,
    asked: bool,
}

/// The parts' pictures: made one at a time beside the window, then kept as textures.
#[derive(Default)]
struct Pictures {
    textures: HashMap<String, Option<egui::TextureHandle>>,
    requested: BTreeSet<String>,
    asked: BTreeSet<String>,
    tx: Option<Sender<PictureRequest>>,
    rx: Option<Receiver<(String, Option<egui::ColorImage>)>>,
}

impl Pictures {
    fn request(&mut self, workers: &Workers, file: &str, entry: &Entry, asked: bool) {
        if self.textures.get(file).is_some_and(Option::is_some)
            || (!asked && self.requested.contains(file))
            || (asked && !self.asked.insert(file.to_string()))
        {
            return;
        }
        if self.tx.is_none() {
            let (tx, requests) = std::sync::mpsc::channel::<PictureRequest>();
            let (done, rx) = std::sync::mpsc::channel();
            if !workers.spawn("parts-library-pictures", move |stop| {
                while let Ok(r) = requests.recv() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let picture = make_picture(&r, &stop);
                    if done.send((r.file, picture)).is_err() {
                        break;
                    }
                }
            }) {
                return;
            }
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        self.requested.insert(file.to_string());
        let request = PictureRequest { file: file.to_string(), kind: entry.kind, bytes: entry.bytes, local: entry.local, asked };
        if self.tx.as_ref().is_some_and(|tx| tx.send(request).is_err()) {
            self.tx = None;
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        while let Ok((file, picture)) = rx.try_recv() {
            let texture = picture.map(|p| ctx.load_texture(format!("part:{file}"), p, egui::TextureOptions::LINEAR));
            self.textures.insert(file, texture);
        }
        if !self.requested.iter().all(|f| self.textures.contains_key(f)) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

/// A part's picture: a design's own, or a CAD file's, drawn once and cached on this computer.
fn make_picture(r: &PictureRequest, stop: &AtomicBool) -> Option<egui::ColorImage> {
    match r.kind {
        Kind::Design => crate::home::decode_thumbnail(solidcraft_engine::io::read_design_thumbnail_file(&r.file)?),
        Kind::Cad => {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(picture) = crate::home::cached(&r.file) {
                return Some(picture);
            }
            if !r.asked && (!r.local || r.bytes > AUTO_PICTURE_BYTES) {
                return None;
            }
            let s = open_cad(&r.file, stop)?;
            crate::home::remember_file(&s, &r.file)
        }
    }
}

/// A session holding the CAD file `file`, read by a helper process (`solidcraft_engine::helper`):
/// the helper saves what it read as a design, and this process opens that, so a file that crashes
/// the geometry kernel fails only its picture. Without helpers it is read here.
fn open_cad(file: &str, stop: &AtomicBool) -> Option<solidcraft_engine::Session> {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let read = std::env::temp_dir().join(format!(
        "solidcraft-part-picture-{}-{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        solidcraft_engine::io::DESIGN_EXT
    ));
    let read = read.to_str()?.to_string();
    let saved = solidcraft_engine::helper::run(&[("doc.open", json!({"path": file})), ("file.save", json!({"path": read}))], stop);
    let mut s = solidcraft_engine::Session::default();
    let opened = saved.is_ok() && s.execute("doc.open", &json!({"path": read})).is_ok();
    let _ = std::fs::remove_file(&read);
    opened.then_some(s)
}

/// A part's library properties, guessed from its names for a conversion.
fn convert_params(entry: &Entry, file: &str, stand_up: bool) -> Value {
    let mut p = json!({"path": file, "name": entry.name, "description": entry.name, "stand_up": stand_up});
    if let Some(number) = parts_library::guess_part_number(entry) {
        p["part_number"] = json!(number);
    }
    p
}

/// Converting a CAD part before inserting it.
struct Inserting {
    name: String,
    file: String,
    rx: Receiver<Result<String, solidcraft_engine::helper::Failure>>,
}

/// Converting a category's CAD parts.
struct Converting {
    total: usize,
    done: usize,
    current: String,
    failed: Vec<(String, String)>,
    stop: std::sync::Arc<AtomicBool>,
    rx: Receiver<ConvertEvent>,
    finished: bool,
    library: usize,
}

enum ConvertEvent {
    Started(String),
    Converted,
    Failed(String, String),
    Finished,
}

/// The window's state.
pub struct PartsLibrary {
    /// What the search box holds.
    pub query: String,
    libs: Vec<Loaded>,
    /// The picked category: a library and a folder in it ("" for the whole library); `None`
    /// for every library.
    pub scope: Option<(usize, String)>,
    expanded: BTreeSet<(usize, String)>,
    /// The picked part: its library and its path there.
    pub picked: Option<(usize, String)>,
    /// Pictures instead of a list.
    pub grid: bool,
    pictures: Pictures,
    /// Asking before converting a category: whether to stand up Y-up files.
    pub confirm_convert: Option<bool>,
    converting: Option<Converting>,
    inserting: Option<Inserting>,
    /// The last refusal, or what the last action did.
    pub note: Option<(String, bool)>,
    workers: std::sync::Arc<Workers>,
}

impl PartsLibrary {
    fn new(libraries: &[LibraryRef], workers: std::sync::Arc<Workers>) -> Self {
        Self {
            query: String::new(),
            libs: libraries.iter().cloned().map(|l| Loaded::new(l, &workers)).collect(),
            scope: None,
            expanded: BTreeSet::new(),
            picked: None,
            grid: true,
            pictures: Pictures::default(),
            confirm_convert: None,
            converting: None,
            inserting: None,
            note: None,
            workers,
        }
    }

    /// No library is being read.
    pub fn read(&self) -> bool {
        self.libs.iter().all(|l| l.scanning.is_none())
    }

    fn entry(&self, (lib, path): &(usize, String)) -> Option<(&Library, &Entry)> {
        let library = self.libs.get(*lib)?.library.as_ref()?;
        Some((library, library.entries.iter().find(|e| &e.path == path)?))
    }

    /// The parts in the picked category that match the search, best first, at most `limit`;
    /// and how many matched.
    fn found(&self, limit: usize) -> (Vec<(usize, &Library, &Entry)>, usize) {
        let terms = parts_library::terms(&self.query);
        let mut found = Vec::new();
        for (i, l) in self.libs.iter().enumerate() {
            let Some(library) = &l.library else { continue };
            for e in &library.entries {
                if !in_scope(&self.scope, i, e) {
                    continue;
                }
                if let Some(score) = parts_library::score(e, &terms) {
                    found.push((score, i, library, e));
                }
            }
        }
        let matched = found.len();
        found.sort_by_key(|(s, ..)| std::cmp::Reverse(*s));
        (found.into_iter().take(limit).map(|(_, i, l, e)| (i, l, e)).collect(), matched)
    }

    fn total(&self) -> usize {
        self.libs.iter().filter_map(|l| l.library.as_ref()).map(|l| l.entries.len()).sum()
    }
}

fn in_scope(scope: &Option<(usize, String)>, lib: usize, entry: &Entry) -> bool {
    match scope {
        None => true,
        Some((l, folder)) => *l == lib && (folder.is_empty() || entry.folder == *folder || entry.folder.starts_with(&format!("{folder}/"))),
    }
}

/// Open the window on the libraries chosen before.
pub fn open(app: &mut SolveApp) {
    adopt_legacy_folder(app);
    app.parts_library = Some(PartsLibrary::new(&app.preferences.parts_libraries, app.parts_workers.clone()));
}

/// The one folder the first Parts Library kept becomes the first library of the list.
fn adopt_legacy_folder(app: &mut SolveApp) {
    let folder = std::mem::take(&mut app.preferences.parts_library_folder);
    if !folder.is_empty() && !app.preferences.parts_libraries.iter().any(|l| l.folder == folder) {
        app.preferences.parts_libraries.push(LibraryRef { name: folder_name(&folder), folder });
    }
}

fn folder_name(folder: &str) -> String {
    std::path::Path::new(folder).file_name().and_then(|n| n.to_str()).filter(|n| !n.is_empty()).unwrap_or(folder).to_string()
}

/// Add `folder` to the libraries and read it.
pub fn add_library(app: &mut SolveApp, folder: String) {
    if app.preferences.parts_libraries.iter().any(|l| l.folder == folder) {
        return;
    }
    let lib = LibraryRef { name: folder_name(&folder), folder };
    app.preferences.parts_libraries.push(lib.clone());
    if let Some(w) = app.parts_library.as_mut() {
        let loaded = Loaded::new(lib, &w.workers);
        w.libs.push(loaded);
        w.scope = Some((w.libs.len() - 1, String::new()));
        w.picked = None;
    }
}

/// Take a library out of the list; its files stay where they are.
pub fn remove_library(app: &mut SolveApp, index: usize) {
    if index < app.preferences.parts_libraries.len() {
        app.preferences.parts_libraries.remove(index);
    }
    if let Some(w) = app.parts_library.as_mut()
        && index < w.libs.len()
    {
        w.libs.remove(index);
        w.scope = None;
        w.picked = None;
        w.expanded.clear();
    }
}

fn kind_label(entry: &Entry) -> String {
    match entry.kind {
        Kind::Design => "SolidCraft".into(),
        Kind::Cad => entry.format.to_uppercase(),
    }
}

/// Link a library design into the open design, as the toolbar's Insert Design does (a library
/// on a server is the user's choice, like a file picked in a dialog).
fn link(app: &mut SolveApp, file: &str) -> Result<(), String> {
    let hosts: Vec<String> = solidcraft_engine::io::vfs::network_host(file).into_iter().collect();
    solidcraft_engine::io::vfs::allowing_hosts(&hosts, || app.run("component.insert_design", json!({"path": file})))?;
    app.fit_view();
    Ok(())
}

/// Insert the picked part: a design is linked at once; a CAD part is saved as a library part
/// beside it first (beside the window), then linked when that is done.
fn insert(app: &mut SolveApp, w: &mut PartsLibrary, picked: &(usize, String)) {
    let Some((library, entry)) = w.entry(picked) else { return };
    let file = library.absolute(entry).to_string_lossy().into_owned();
    let name = entry.name.clone();
    match entry.kind {
        Kind::Design => {
            w.note = Some(match link(app, &file) {
                Ok(()) => (format("ui.parts_library_inserted", &[("name", &name)]), false),
                Err(e) => (e, true),
            });
        }
        Kind::Cad => {
            let params = convert_params(entry, &file, app.preferences.parts_library_stand_up);
            let (tx, rx) = std::sync::mpsc::channel();
            let started = w.workers.spawn("parts-library-convert", move |stop| {
                let r = solidcraft_engine::helper::run(&[("library.parts.convert", params)], &stop);
                let _ = tx.send(r.and_then(|r| {
                    r["file"]
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| solidcraft_engine::helper::Failure { message: "no file".into(), crashed: false })
                }));
            });
            if started {
                w.note = Some((format("ui.parts_library_preparing", &[("name", &name)]), false));
                w.inserting = Some(Inserting { name, file, rx });
            } else {
                app.insert_path_async(&file);
            }
        }
    }
}

/// Finish an insert whose library part is ready: link it, or insert a copy of the CAD file
/// when the part couldn't be made (a library the user can't write to).
fn poll_insert(app: &mut SolveApp, w: &mut PartsLibrary) {
    let Some(job) = &w.inserting else { return };
    let Ok(result) = job.rx.try_recv() else { return };
    let Some(job) = w.inserting.take() else { return };
    match result {
        Ok(design) => {
            w.note = Some(match link(app, &design) {
                Ok(()) => (format("ui.parts_library_saved_and_inserted", &[("name", &job.name)]), false),
                Err(e) => (e, true),
            });
            if let Some((lib, _)) = &w.picked
                && let Some(l) = w.libs.get_mut(*lib)
            {
                l.rescan(&w.workers);
            }
        }
        // The file crashed the helper reading it: reading it here would close the window.
        Err(e) if e.crashed => {
            w.note = Some((format("ui.parts_library_not_read", &[("name", &job.name), ("reason", &e.message)]), true));
        }
        Err(e) => {
            app.insert_path_async(&job.file);
            w.note = Some((format("ui.parts_library_copied_instead", &[("name", &job.name), ("reason", &e.message)]), true));
        }
    }
}

/// Convert every CAD part in the picked category, one after another beside the window.
fn start_converting(w: &mut PartsLibrary, stand_up: bool) {
    let Some((lib, _)) = &w.scope else { return };
    let lib = *lib;
    let Some(library) = w.libs.get(lib).and_then(|l| l.library.as_ref()) else { return };
    let jobs: Vec<(String, Value)> = library
        .entries
        .iter()
        .filter(|e| e.kind == Kind::Cad && in_scope(&w.scope, lib, e))
        .map(|e| {
            let file = library.absolute(e).to_string_lossy().into_owned();
            (e.name.clone(), convert_params(e, &file, stand_up))
        })
        .collect();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let flag = stop.clone();
    let total = jobs.len();
    if !w.workers.spawn("parts-library-convert-all", move |quitting| {
        for (name, params) in jobs {
            if flag.load(Ordering::Relaxed) || quitting.load(Ordering::Relaxed) {
                break;
            }
            let _ = tx.send(ConvertEvent::Started(name.clone()));
            let event = match solidcraft_engine::helper::run(&[("library.parts.convert", params)], &quitting) {
                Ok(_) => ConvertEvent::Converted,
                Err(e) => ConvertEvent::Failed(name, e.message),
            };
            let _ = tx.send(event);
        }
        let _ = tx.send(ConvertEvent::Finished);
    }) {
        return;
    }
    w.converting = Some(Converting { total, done: 0, current: String::new(), failed: Vec::new(), stop, rx, finished: false, library: lib });
}

fn poll_converting(w: &mut PartsLibrary, ctx: &egui::Context) {
    let Some(c) = w.converting.as_mut() else { return };
    let mut rescan = false;
    while let Ok(event) = c.rx.try_recv() {
        match event {
            ConvertEvent::Started(name) => c.current = name,
            ConvertEvent::Converted => c.done += 1,
            ConvertEvent::Failed(name, reason) => {
                c.done += 1;
                c.failed.push((name, reason));
            }
            ConvertEvent::Finished => {
                c.finished = true;
                rescan = true;
            }
        }
    }
    if !c.finished {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    let lib = c.library;
    if rescan && let Some(l) = w.libs.get_mut(lib) {
        l.rescan(&w.workers);
    }
}

/// Folder path → number of parts in it and below, for each library ("" is the library).
fn counts(library: &Library) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for e in &library.entries {
        *counts.entry(String::new()).or_insert(0) += 1;
        let mut prefix = String::new();
        for part in e.folder.split('/').filter(|p| !p.is_empty()) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            *counts.entry(prefix.clone()).or_insert(0) += 1;
        }
    }
    counts
}

fn parent(folder: &str) -> &str {
    folder.rsplit_once('/').map_or("", |(p, _)| p)
}

/// One row of the category tree; `true` when it was clicked.
fn tree_row(ui: &mut egui::Ui, key: &str, depth: usize, label: &str, count: usize, has_children: bool, open: bool, picked: bool) -> (bool, bool) {
    let t = Tokens::get();
    let height = crate::ui_kit::row_height();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::click());
    if picked || response.hovered() {
        ui.painter().rect_filled(rect, t.control_radius, if picked { t.selected_bg } else { t.hover });
    }
    let indent = rect.left() + t.space_8 + depth as f32 * t.space_12;
    let mut toggled = false;
    if has_children {
        let toggle = egui::Rect::from_center_size(egui::pos2(indent + t.space_6, rect.center().y), vec2(t.space_12, height));
        if crate::ui_kit::disclosure_button_at(ui, toggle, open, label).clicked() {
            toggled = true;
        }
    }
    let text_rect = egui::Rect::from_min_max(egui::pos2(indent + t.space_16, rect.top()), rect.max);
    let text = format!("{label}  {count}");
    ui.painter().with_clip_rect(text_rect).text(
        egui::pos2(text_rect.left(), text_rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(t.font_label),
        if picked { t.text } else { t.text_dim },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, picked, label));
    crate::scenario::publish_rect(&format!("dialog:list:parts_library_tree:{key}"), rect);
    (response.clicked() && !toggled, toggled)
}

/// The category tree: every library, then a library's folders under it.
fn tree(ui: &mut egui::Ui, w: &mut PartsLibrary) {
    let total = w.total();
    if tree_row(ui, "all", 0, tr("ui.parts_library_all"), total, false, false, w.scope.is_none()).0 {
        w.scope = None;
    }
    for i in 0..w.libs.len() {
        let Some(library) = w.libs[i].library.as_ref() else {
            let name = w.libs[i].lib.name.clone();
            let _ = tree_row(ui, &format!("{i}"), 0, &name, 0, false, false, w.scope == Some((i, String::new())));
            continue;
        };
        let counts = counts(library);
        let mut rows: Vec<(String, usize)> = vec![(String::new(), 0)];
        // Depth-first: a folder's sub-folders right below it, while it is open.
        let mut stack: Vec<String> = counts.keys().filter(|k| !k.is_empty() && !k.contains('/')).rev().cloned().collect();
        let lib_open = w.expanded.contains(&(i, String::new()));
        if lib_open {
            while let Some(folder) = stack.pop() {
                let depth = folder.matches('/').count() + 1;
                rows.push((folder.clone(), depth));
                if w.expanded.contains(&(i, folder.clone())) {
                    let mut children: Vec<String> =
                        counts.keys().filter(|k| k.starts_with(&format!("{folder}/")) && parent(k) == folder).cloned().collect();
                    children.reverse();
                    stack.extend(children);
                }
            }
        }
        for (folder, depth) in rows {
            let has_children =
                if folder.is_empty() { counts.len() > 1 } else { counts.keys().any(|k| parent(k) == folder && !k.is_empty() && k != &folder) };
            let label = if folder.is_empty() { w.libs[i].lib.name.clone() } else { folder.rsplit('/').next().unwrap_or(&folder).to_string() };
            let key = (i, folder.clone());
            let open = w.expanded.contains(&key);
            let count = counts.get(&folder).copied().unwrap_or(0);
            let (clicked, toggled) = tree_row(ui, &format!("{i}:{folder}"), depth, &label, count, has_children, open, w.scope.as_ref() == Some(&key));
            if toggled && !w.expanded.remove(&key) {
                w.expanded.insert(key.clone());
            }
            if clicked {
                if has_children && !open {
                    w.expanded.insert(key.clone());
                }
                w.scope = Some(key);
            }
        }
    }
}

/// One picture card; its response.
fn card(ui: &mut egui::Ui, w: &mut PartsLibrary, lib: usize, library: &Library, entry: &Entry) -> egui::Response {
    let t = Tokens::get();
    let [tw, th] = solidcraft_engine::io::THUMBNAIL_SIZE;
    let picture_h = CARD_WIDTH * th as f32 / tw as f32;
    let (rect, response) = ui.allocate_exact_size(vec2(CARD_WIDTH, picture_h + t.space_8 + crate::ui_kit::row_height() * 1.6), egui::Sense::click());
    let picked = w.picked.as_ref().is_some_and(|(l, p)| *l == lib && *p == entry.path);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, picked, &entry.name));
    crate::scenario::publish_rect(&format!("dialog:list:parts_library:{}", entry.path), rect);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let file = library.absolute(entry).to_string_lossy().into_owned();
    w.pictures.request(&w.workers, &file, entry, false);
    let painter = ui.painter();
    if picked || response.hovered() {
        painter.rect_filled(rect, t.control_radius, if picked { t.selected_bg } else { t.hover });
    }
    let picture = egui::Rect::from_min_size(rect.min + vec2(t.space_4, t.space_4), vec2(CARD_WIDTH - t.space_8, picture_h - t.space_4));
    match w.pictures.textures.get(&file) {
        Some(Some(texture)) => {
            painter.image(texture.id(), picture, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), t.file_image_tint);
        }
        _ => {
            // Not drawn yet, or (the download icon) kept only in the cloud until it is clicked.
            let icon = if entry.local || entry.kind == Kind::Design { "component" } else { "import" };
            crate::icons::paint_ink(painter, egui::Rect::from_center_size(picture.center(), vec2(t.space_24, t.space_24)), icon, t.text_dim);
        }
    }
    let label = egui::Rect::from_min_max(egui::pos2(rect.left() + t.space_4, picture.bottom() + t.space_4), rect.max - vec2(t.space_4, 0.0));
    let galley = painter.layout(entry.name.clone(), egui::FontId::proportional(t.font_caption), t.text, label.width());
    let galley = if galley.rows.len() > 2 {
        let mut job = egui::text::LayoutJob::simple(entry.name.clone(), egui::FontId::proportional(t.font_caption), t.text, label.width());
        job.wrap.max_rows = 2;
        painter.layout_job(job)
    } else {
        galley
    };
    painter.galley(label.min, galley, t.text);
    response
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut w) = app.parts_library.take() else { return };
    if w.libs.iter_mut().map(Loaded::poll).fold(false, |a, b| a | b) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    w.pictures.poll(ctx);
    poll_insert(app, &mut w);
    poll_converting(&mut w, ctx);
    let t = Tokens::get();
    let (mut add, mut remove, mut refresh, mut insert_picked, mut start_convert) = (false, None, false, false, None);
    let stand_up_default = app.preferences.parts_library_stand_up;
    let closed = crate::ui_kit::Window::new(egui::Id::new("sc_parts_library"), "folder", tr("cmd.library.parts.search")).cards(10.0 / 3.0).show_body(
        ctx,
        |ui| {
            ui.horizontal_wrapped(|ui| {
                if crate::ui_kit::button(ui, tr("ui.parts_library_add"), crate::ui_kit::Button::Normal).clicked() {
                    add = true;
                }
                let library_picked = w.scope.as_ref().filter(|(_, f)| f.is_empty()).map(|(l, _)| *l);
                let button = ui
                    .add_enabled_ui(library_picked.is_some(), |ui| {
                        crate::ui_kit::button(ui, tr("ui.parts_library_remove"), crate::ui_kit::Button::Normal)
                    })
                    .inner;
                if button.on_hover_text(tr("ui.parts_library_remove_hint")).clicked() {
                    remove = library_picked;
                }
                if !w.libs.is_empty() && crate::ui_kit::button(ui, tr("ui.parts_library_refresh"), crate::ui_kit::Button::Normal).clicked() {
                    refresh = true;
                }
                let mut view = usize::from(!w.grid);
                let labels = [tr("ui.parts_library_grid"), tr("ui.parts_library_list")];
                let width = crate::ui_kit::ContentWidth::button(ui, labels[0]).max(crate::ui_kit::ContentWidth::button(ui, labels[1])) * 2.0;
                ui.allocate_ui(vec2(width, crate::ui_kit::row_height()), |ui| {
                    crate::ui_kit::segmented(ui, &mut view, &labels, Default::default());
                });
                w.grid = view == 0;
            });
            if w.libs.is_empty() {
                ui.label(tr("ui.parts_library_none"));
                return;
            }
            crate::ui_kit::search_field(ui, egui::Id::new("sc_parts_library_search"), &mut w.query, tr("ui.parts_library_search"));
            let height = crate::ui_kit::table_row_height() * 13.0;
            ui.horizontal_top(|ui| {
                ui.allocate_ui(vec2(TREE_WIDTH, height), |ui| {
                    ui.set_width(TREE_WIDTH);
                    egui::ScrollArea::vertical()
                        .id_salt("sc_parts_library_tree")
                        .max_height(height)
                        .show(ui, |ui| ui.vertical(|ui| tree(ui, &mut w)));
                });
                ui.separator();
                ui.vertical(|ui| {
                    if !w.read() {
                        ui.label(RichText::new(tr("ui.parts_library_reading")).color(t.text_dim));
                    }
                    for l in &w.libs {
                        if let Some(e) = &l.error {
                            ui.label(RichText::new(format!("{}: {e}", l.lib.name)).small().color(t.error));
                        }
                    }
                    let (found, matched) = w.found(MAX_ROWS);
                    let found: Vec<(usize, Library, Entry)> =
                        found.into_iter().map(|(i, l, e)| (i, Library { root: l.root.clone(), ..Default::default() }, e.clone())).collect();
                    let cad_here = match &w.scope {
                        Some((lib, _)) => w
                            .libs
                            .get(*lib)
                            .and_then(|l| l.library.as_ref())
                            .map_or(0, |library| library.entries.iter().filter(|e| e.kind == Kind::Cad && in_scope(&w.scope, *lib, e)).count()),
                        None => 0,
                    };
                    let entry = w.picked.clone().and_then(|p| w.entry(&p).map(|(_, e)| e.clone()));
                    ui.horizontal_wrapped(|ui| {
                        let busy = w.inserting.is_some();
                        let button = ui
                            .add_enabled_ui(entry.is_some() && !busy, |ui| {
                                crate::ui_kit::button(ui, tr("ui.parts_library_insert"), crate::ui_kit::Button::Normal)
                            })
                            .inner;
                        if button.clicked() {
                            insert_picked = true;
                        }
                        if cad_here > 0 && w.converting.as_ref().is_none_or(|c| c.finished) {
                            let label = format("ui.parts_library_convert", &[("count", &cad_here.to_string())]);
                            if crate::ui_kit::button(ui, &label, crate::ui_kit::Button::Normal).clicked() {
                                w.confirm_convert = Some(stand_up_default);
                            }
                        }
                        let shown = found.len().min(matched).to_string();
                        let total = w.total().to_string();
                        ui.label(RichText::new(format("ui.parts_library_count", &[("shown", &shown), ("total", &total)])).color(t.text_dim));
                    });
                    if let Some(entry) = &entry {
                        let hint = match entry.kind {
                            Kind::Design => "ui.parts_library_design_hint",
                            Kind::Cad => "ui.parts_library_cad_hint",
                        };
                        ui.label(RichText::new(tr(hint)).small().color(t.text_dim));
                    }
                    if let Some(mut stand_up) = w.confirm_convert {
                        ui.separator();
                        ui.label(format("ui.parts_library_convert_explain", &[("count", &cad_here.to_string())]));
                        crate::ui_kit::checkbox(ui, tr("ui.parts_library_stand_up"), &mut stand_up);
                        let mut cancel = false;
                        ui.horizontal(|ui| {
                            if crate::ui_kit::button(ui, tr("ui.parts_library_convert_start"), crate::ui_kit::Button::Normal).clicked() {
                                start_convert = Some(stand_up);
                            }
                            cancel = crate::ui_kit::button(ui, tr("ui.parts_library_cancel"), crate::ui_kit::Button::Normal).clicked();
                        });
                        w.confirm_convert = (!cancel).then_some(stand_up);
                        ui.separator();
                    }
                    if let Some(c) = &w.converting {
                        let (done, total) = (c.done.to_string(), c.total.to_string());
                        if c.finished {
                            let ok = (c.done - c.failed.len()).to_string();
                            ui.label(format("ui.parts_library_converted", &[("ok", &ok), ("total", &total)]));
                        } else {
                            ui.horizontal(|ui| {
                                ui.label(format("ui.parts_library_converting", &[("done", &done), ("total", &total), ("name", &c.current)]));
                                if crate::ui_kit::button(ui, tr("ui.parts_library_stop"), crate::ui_kit::Button::Normal).clicked() {
                                    c.stop.store(true, Ordering::Relaxed);
                                }
                            });
                            crate::ui_kit::progress(ui, "parts_library_convert", c.done as f32 / c.total.max(1) as f32);
                        }
                        if !c.failed.is_empty() {
                            ui.label(RichText::new(format("ui.parts_library_failed", &[("count", &c.failed.len().to_string())])).color(t.error));
                            egui::ScrollArea::vertical().id_salt("sc_parts_library_failed").max_height(crate::ui_kit::row_height() * 3.0).show(
                                ui,
                                |ui| {
                                    for (name, reason) in &c.failed {
                                        ui.label(RichText::new(format!("{name}: {reason}")).small().color(t.text_dim));
                                    }
                                },
                            );
                        }
                    }
                    if let Some((note, error)) = &w.note {
                        ui.label(RichText::new(note).color(if *error { t.error } else { t.text_dim }));
                    }
                    let mut picked_now = None;
                    egui::ScrollArea::vertical().id_salt("sc_parts_library_rows").max_height(height).show(ui, |ui| {
                        if found.is_empty() && w.read() {
                            ui.label(RichText::new(tr("ui.parts_library_no_match")).color(t.text_dim));
                        }
                        if w.grid {
                            ui.horizontal_wrapped(|ui| {
                                for (lib, library, entry) in &found {
                                    let r = card(ui, &mut w, *lib, library, entry).on_hover_text(&entry.path);
                                    if r.clicked() || r.double_clicked() {
                                        picked_now = Some(((*lib, entry.path.clone()), r.double_clicked()));
                                    }
                                }
                            });
                        } else {
                            let columns = [
                                Column::new(tr("ui.parts_library_col_name"), 1.5).flex(),
                                Column::new(tr("ui.parts_library_col_number"), 0.8).flex(),
                                Column::new(tr("ui.parts_library_col_folder"), 1.8).flex(),
                                Column::new(tr("ui.parts_library_col_type"), 0.6),
                            ];
                            crate::ui_kit::table_custom(ui, "parts_library", &columns, tr("ui.parts_library_no_match"), |ui, table| {
                                for (lib, _, entry) in &found {
                                    let picked = w.picked.as_ref().is_some_and(|(l, p)| l == lib && *p == entry.path);
                                    // Plain text, not selectable: a click anywhere on the row picks the part.
                                    let cell = |ui: &mut egui::Ui, text: RichText| ui.add(egui::Label::new(text).selectable(false).truncate());
                                    let row = table.row(ui, &entry.path, picked, |row| {
                                        row.cell(|ui| cell(ui, RichText::new(&entry.name)));
                                        row.cell(|ui| cell(ui, RichText::new(entry.part_number.as_deref().unwrap_or(""))));
                                        row.cell(|ui| cell(ui, RichText::new(entry.folder.replace('/', " › ")).color(t.text_dim)));
                                        row.cell(|ui| cell(ui, RichText::new(kind_label(entry)).color(t.text_dim)));
                                    });
                                    if row.clicked() || row.double_clicked() {
                                        picked_now = Some(((*lib, entry.path.clone()), row.double_clicked()));
                                    }
                                    row.on_hover_text(&entry.path);
                                }
                            });
                        }
                    });
                    if let Some((picked, double)) = picked_now {
                        // A click on a part asks for its picture even if it is large or in the cloud.
                        if let Some((library, entry)) = w.entry(&picked).map(|(l, e)| (l.root.clone(), e.clone())) {
                            let file = Library { root: library, ..Default::default() }.absolute(&entry).to_string_lossy().into_owned();
                            let workers = w.workers.clone();
                            w.pictures.request(&workers, &file, &entry, true);
                        }
                        w.picked = Some(picked);
                        insert_picked |= double;
                    }
                });
            });
        },
    );
    if refresh {
        let workers = w.workers.clone();
        for l in &mut w.libs {
            l.rescan(&workers);
        }
    }
    if let Some(stand_up) = start_convert {
        app.preferences.parts_library_stand_up = stand_up;
        w.confirm_convert = None;
        start_converting(&mut w, stand_up);
    }
    if insert_picked && let Some(picked) = w.picked.clone() {
        insert(app, &mut w, &picked);
    }
    if closed {
        if let Some(c) = &w.converting {
            c.stop.store(true, Ordering::Relaxed);
        }
    } else {
        app.parts_library = Some(w);
    }
    if let Some(index) = remove {
        remove_library(app, index);
    }
    if add {
        app.pick_folder(tr("ui.parts_library_choose_title"), |app, folder| {
            if let Some(folder) = folder {
                add_library(app, folder);
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
    fn library(h: &Harness, name: &str, files: &[(&str, Option<&std::path::Path>)]) -> String {
        let root = h.dir.join(name);
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

    fn wait(h: &mut Harness, done: impl Fn(&super::PartsLibrary) -> bool) {
        for _ in 0..1000 {
            h.frames(1);
            if h.app.parts_library.as_ref().is_some_and(&done) {
                h.frames(2);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the window never got there");
    }

    #[test]
    fn libraries_are_added_listed_by_category_and_searched() {
        let mut h = Harness::new();
        let team = library(
            &h,
            "ROV PARTS",
            &[("THRUSTERS/BLUE ROBOTICS/T200/T200.STEP", None), ("CAMERAS/Low-light camera.igs", None), ("CAMERAS/notes.pdf", None)],
        );
        let mine = library(&h, "My Parts", &[("Brackets/Bracket.stp", None)]);
        h.step(&json!({"start": "library.parts.search"})).unwrap();
        assert!(h.app.parts_library.is_some(), "ui.start opens the window");
        h.frames(2);
        assert!(h.widget("Add Library…").is_some());
        super::add_library(&mut h.app, team.clone());
        super::add_library(&mut h.app, mine.clone());
        wait(&mut h, super::PartsLibrary::read);
        assert_eq!(
            h.app.preferences.parts_libraries.iter().map(|l| (l.name.as_str(), l.folder.as_str())).collect::<Vec<_>>(),
            [("ROV PARTS", team.as_str()), ("My Parts", mine.as_str())],
            "the libraries are remembered"
        );
        h.app.parts_library.as_mut().unwrap().scope = None;
        h.app.parts_library.as_mut().unwrap().grid = false;
        h.frames(2);
        assert!(h.widget("3 of 3 parts").is_some(), "every library");
        assert!(h.widget("T200").is_some() && h.widget("Low-light camera").is_some() && h.widget("Bracket").is_some());
        // A category: the tree's folder rows.
        h.step(&json!({"widget": "ROV PARTS"})).unwrap();
        h.frames(2);
        h.step(&json!({"widget": "CAMERAS"})).unwrap();
        h.frames(2);
        assert_eq!(h.app.parts_library.as_ref().unwrap().scope, Some((0, "CAMERAS".to_string())));
        assert!(h.widget("1 of 3 parts").is_some());
        assert!(h.widget("T200").is_none());
        h.app.parts_library.as_mut().unwrap().scope = None;
        h.app.parts_library.as_mut().unwrap().query = "blue robotics".into();
        h.frames(2);
        assert!(h.widget("1 of 3 parts").is_some(), "the vendor's folder is searched");
        h.app.parts_library.as_mut().unwrap().query = "sonar".into();
        h.frames(2);
        assert!(h.widget("No parts match your search.").is_some());
        // Remove takes the library out of the list, not off the disk.
        super::remove_library(&mut h.app, 1);
        assert_eq!(h.app.preferences.parts_libraries.len(), 1);
        assert!(std::path::Path::new(&mine).join("Brackets/Bracket.stp").is_file());
    }

    #[test]
    fn the_first_parts_library_folder_becomes_the_first_library() {
        let mut h = Harness::new();
        let folder = library(&h, "LIBRARIES", &[("A/Part.step", None)]);
        h.app.preferences.parts_library_folder = folder.clone();
        h.step(&json!({"start": "library.parts.search"})).unwrap();
        assert_eq!(h.app.preferences.parts_libraries, [super::LibraryRef { name: "LIBRARIES".into(), folder }]);
        assert!(h.app.preferences.parts_library_folder.is_empty());
    }

    #[test]
    fn insert_links_a_library_design() {
        let mut h = Harness::new();
        h.app.run("solid.box", json!({"length": 40, "width": 20, "height": 3})).unwrap();
        h.app.run("document.metadata.set", json!({"values": {"part_number": "PL-100", "description": "Mounting plate"}})).unwrap();
        let part = h.dir.join("plate.solidcraft");
        h.app.run("file.save", json!({"path": part.to_string_lossy()})).unwrap();
        crate::documents::new_design(&mut h.app);
        let folder = library(&h, "LIBRARY", &[("MOUNTING HARDWARE/Plate.solidcraft", Some(&part))]);
        h.app.preferences.parts_libraries = vec![super::LibraryRef { name: "Library".into(), folder }];
        h.step(&json!({"start": "library.parts.search"})).unwrap();
        wait(&mut h, super::PartsLibrary::read);
        h.app.parts_library.as_mut().unwrap().grid = false;
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

    #[test]
    fn inserting_a_step_part_saves_it_as_a_library_part_and_links_it_and_convert_does_a_category() {
        let mut h = Harness::new();
        // Two STEP parts, made here.
        let mut maker = solidcraft_engine::Session::default();
        maker.execute("solid.box", &json!({"length": 40, "width": 20, "height": 10})).unwrap();
        let step = h.dir.join("box.step");
        maker.execute("file.export", &json!({"path": step.to_str().unwrap()})).unwrap();
        let folder = library(
            &h,
            "LIBRARY",
            &[("THRUSTERS/T200_THRUSTER_BR-100545/T200.step", Some(&step)), ("THRUSTERS/T500_THRUSTER_BR-101092/T500.step", Some(&step))],
        );
        h.app.preferences.parts_libraries = vec![super::LibraryRef { name: "Library".into(), folder: folder.clone() }];
        h.step(&json!({"start": "library.parts.search"})).unwrap();
        wait(&mut h, super::PartsLibrary::read);
        let w = h.app.parts_library.as_mut().unwrap();
        w.picked = Some((0, "THRUSTERS/T200_THRUSTER_BR-100545/T200.step".into()));
        h.frames(2);
        h.step(&json!({"widget": "Insert Part"})).unwrap();
        wait(&mut h, |w| w.inserting.is_none());
        wait(&mut h, super::PartsLibrary::read);
        let design = std::path::Path::new(&folder).join("THRUSTERS/T200_THRUSTER_BR-100545/T200.sc");
        assert!(design.is_file(), "the library part is saved beside the STEP");
        assert_eq!(h.app.session.doc.links.len(), 1, "and linked: {:?}", h.app.parts_library.as_ref().unwrap().note);
        let doc = solidcraft_engine::io::read_design(&std::fs::read(&design).unwrap()).unwrap();
        assert_eq!((doc.name.as_str(), doc.metadata.0.get("part_number")), ("T200 THRUSTER BR-100545", Some(&json!("BR-100545"))));
        let w = h.app.parts_library.as_ref().unwrap();
        let kinds: Vec<super::Kind> = w.libs[0].library.as_ref().unwrap().entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [super::Kind::Design, super::Kind::Cad], "the design replaces its STEP in the list");
        // Convert the category: the one STEP part left.
        h.app.parts_library.as_mut().unwrap().scope = Some((0, "THRUSTERS".into()));
        h.frames(2);
        h.step(&json!({"widget": "Convert 1 to SolidCraft Parts…"})).unwrap();
        h.frames(2);
        h.step(&json!({"widget": "Start Converting"})).unwrap();
        wait(&mut h, |w| w.converting.as_ref().is_some_and(|c| c.finished));
        wait(&mut h, super::PartsLibrary::read);
        assert!(std::path::Path::new(&folder).join("THRUSTERS/T500_THRUSTER_BR-101092/T500.sc").is_file());
        assert!(h.widget("Converted 1 of 1 parts.").is_some());
        let entries = &h.app.parts_library.as_ref().unwrap().libs[0].library.as_ref().unwrap().entries;
        assert!(entries.iter().all(|e| e.kind == super::Kind::Design));
    }
}
