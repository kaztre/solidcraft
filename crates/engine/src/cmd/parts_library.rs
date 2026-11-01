//! Parts libraries (docs/parts-library.md): `library.parts.search` lists and searches the parts
//! in a library folder, and `library.parts.convert` saves a STEP or IGES part as a SolidCraft
//! library part beside it, so it can be linked and shows its picture. The Parts Library window
//! (`ui.start` of `library.parts.search`) searches the same list, converts a CAD part the first
//! time it is inserted, and links it with `component.insert_design`.

use serde_json::{Value, json};
use solidcraft_io::parts_library::{self, Kind};

use super::CommandSpec;
use crate::params::bad;
use crate::schema::parts_library::{LibraryPart, PartsConvertParams, PartsConvertReply, PartsSearchParams, PartsSearchReply};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const SEARCH_CONTRACT: TypedContract<PartsSearchParams> = TypedContract::new();
const CONVERT_CONTRACT: TypedContract<PartsConvertParams> = TypedContract::new();

/// Parts returned when the call names no limit, and the most it may name.
const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 5000;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("library.parts.search", "Parts Library", crate::checked!("library.parts.search", search))
        .at("SOLID", "INSERT")
        .icon("folder")
        .noundo()
        .contract(&SEARCH_CONTRACT)
        .output(output_schema_for::<PartsSearchReply>)
        .params(
            "folder: the library folder (its sub-folders are categories; a part's STEP, IGES and design files with one name are one \
         part, the design first; meshes, drawings and SolidWorks files are left out); query?: words that must all appear in a \
         part's name, folders, file type, part number or description (default: every part); limit?: most parts returned \
         (default 200, at most 5000) → folder, parts: [{path, file, name, folder, kind: design|cad, format, bytes, part_number?, \
         description?, revision?}] best first, matched, total, truncated. Never changes the design",
        ),
    CommandSpec::new("library.parts.convert", "Convert to Library Part", crate::checked!("library.parts.convert", convert))
        .icon("import")
        .noundo()
        .contract(&CONVERT_CONTRACT)
        .output(output_schema_for::<PartsConvertReply>)
        .params(
            "path: a STEP, IGES or other CAD file SolidCraft reads; name?: the part's name (default the file's name); part_number?, \
             description?, organization? (who makes it): its properties; stand_up?: bool (default false: the file is kept as it is; \
             true: a file whose header names a Y-up exporter (SolidWorks) is turned a quarter turn about X, Y becoming Z); \
             overwrite?: bool (default false: a design already beside the file is refused) → file (the design saved beside the CAD \
             file, with its name and the .solidcraft extension, and its picture), name, bodies, stood_up, exporter_up?. Never changes \
             the open design",
        ),
];

fn search(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "library.parts.search";
    let folder =
        p.get("folder").and_then(Value::as_str).filter(|f| !f.trim().is_empty() && f.len() < 4096).ok_or_else(|| bad(cmd, "`folder` is required"))?;
    let query = match p.get("query") {
        None | Some(Value::Null) => "",
        Some(Value::String(q)) => q.as_str(),
        Some(_) => return Err(bad(cmd, "`query` must be text")),
    };
    let limit = match p.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(v) => v.as_u64().filter(|n| *n > 0).ok_or_else(|| bad(cmd, "`limit` must be a whole number of parts, 1 or more"))?.min(MAX_LIMIT as u64)
            as usize,
    };
    let library = parts_library::scan(folder).map_err(|e| bad(cmd, e.to_string()))?;
    let (found, matched) = parts_library::search(&library, query, limit);
    let parts = found
        .into_iter()
        .map(|e| LibraryPart {
            path: e.path.clone(),
            file: library.absolute(e).to_string_lossy().into_owned(),
            name: e.name.clone(),
            folder: e.folder.clone(),
            kind: match e.kind {
                Kind::Design => "design",
                Kind::Cad => "cad",
            }
            .into(),
            format: e.format.clone(),
            bytes: e.bytes,
            part_number: e.part_number.clone(),
            description: e.description.clone(),
            revision: e.revision.clone(),
        })
        .collect();
    let reply = PartsSearchReply {
        folder: folder.to_string(),
        parts,
        matched: matched as u64,
        total: library.entries.len() as u64,
        truncated: library.truncated,
    };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

/// A text parameter: absent, null or blank is `None`.
fn text<'a>(cmd: &str, p: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(t)) if t.len() <= 4096 => Ok(Some(t.trim()).filter(|t| !t.is_empty())),
        Some(_) => Err(bad(cmd, format!("`{key}` must be text"))),
    }
}

fn flag(cmd: &str, p: &Value, key: &str) -> Result<bool> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad(cmd, format!("`{key}` must be true or false"))),
    }
}

fn convert(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "library.parts.convert";
    let path = text(cmd, p, "path")?.ok_or_else(|| bad(cmd, "`path` is required"))?;
    if !solidcraft_io::is_step_path(path) {
        return Err(bad(cmd, format!("{path} is not a STEP, IGES or other CAD file SolidCraft reads")));
    }
    let source = std::path::Path::new(path);
    let stem = source.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, format!("{path} has no file name")))?;
    let out = source.with_file_name(format!("{stem}.{}", solidcraft_io::DESIGN_EXT));
    let out = out.to_str().ok_or_else(|| bad(cmd, "the design's path is not text"))?.to_string();
    if !flag(cmd, p, "overwrite")? && solidcraft_io::vfs::design_path_is_file(&out, None) == Some(true) {
        return Err(bad(cmd, format!("{out} already exists")));
    }
    let name = text(cmd, p, "name")?.unwrap_or(stem).to_string();
    let stand_up = flag(cmd, p, "stand_up")?;
    let mut values = serde_json::Map::new();
    for key in ["part_number", "description", "organization"] {
        if let Some(v) = text(cmd, p, key)? {
            values.insert(key.into(), json!(v));
        }
    }
    // The part is made in a session of its own: the open design is never touched.
    let mut part = Session::default();
    let opened = part.execute("doc.open", &json!({"path": path}))?;
    let exporter_up = opened.get("exporter_up").and_then(Value::as_str).map(str::to_string);
    let stood_up = stand_up && exporter_up.as_deref() == Some("y");
    if stood_up {
        // As the window's Stand It Up does: the import's component, else its bodies.
        match opened.get("occurrence").and_then(Value::as_u64) {
            Some(occurrence) => {
                part.execute("occurrence.move", &json!({"occurrence": occurrence, "axis": [1, 0, 0], "angle": "90 deg", "move_grounded": true}))?
            }
            None => {
                let bodies: Vec<String> = part.world_state()?.bodies.iter().map(|b| b.name.clone()).collect();
                part.execute("solid.move", &json!({"bodies": bodies, "axis": [1, 0, 0], "angle": "90 deg"}))?
            }
        };
    }
    if !values.is_empty() {
        part.execute("document.metadata.set", &json!({"values": values}))?;
    }
    part.doc_mut().name.clone_from(&name);
    // Display meshes ready, so the save draws the part's picture into the file.
    let world = part.world_state()?;
    let bodies = world.bodies.len();
    world.warm_display_at(solidcraft_doc::display_quality::DisplayQuality::Normal, || true);
    part.execute("file.save", &json!({"path": out}))?;
    let reply = PartsConvertReply { file: out, name, bodies: bodies as u64, stood_up, exporter_up };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use serde_json::json;

    use crate::Session;

    #[test]
    fn a_search_lists_the_matching_parts_and_leaves_the_design_alone() {
        let dir = std::env::temp_dir().join(format!("solidcraft-parts-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in ["THRUSTERS/T200/T200.STEP", "THRUSTERS/T200/T200.STL", "CAMERAS/Low-light camera.igs"] {
            let path = dir.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        let mut s = Session::default();
        let before = s.doc.to_json();
        let folder = dir.to_str().unwrap();
        let r = s.execute("library.parts.search", &json!({"folder": folder, "query": "t200"})).unwrap();
        assert_eq!((r["matched"].as_u64(), r["total"].as_u64(), r["truncated"].as_bool()), (Some(1), Some(2), Some(false)), "{r}");
        let part = &r["parts"][0];
        assert_eq!(
            (&part["name"], &part["folder"], &part["kind"], &part["format"]),
            (&json!("T200"), &json!("THRUSTERS"), &json!("cad"), &json!("step"))
        );
        assert!(std::path::Path::new(part["file"].as_str().unwrap()).is_file());
        let all = s.execute("library.parts.search", &json!({"folder": folder, "limit": 1})).unwrap();
        assert_eq!((all["parts"].as_array().unwrap().len(), all["matched"].as_u64()), (1, Some(2)), "the limit cuts the list, not the count");
        assert_eq!(s.doc.to_json(), before);
        for bad in [json!({}), json!({"folder": ""}), json!({"folder": folder, "limit": 0}), json!({"folder": folder, "query": 3})] {
            assert!(s.execute("library.parts.search", &bad).is_err(), "{bad}");
        }
        assert!(s.execute("library.parts.search", &json!({"folder": dir.join("missing").to_str().unwrap()})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_step_part_becomes_a_library_design_beside_it_with_its_properties_and_picture() {
        crate::capture::install_test_renderer();
        let dir = std::env::temp_dir().join(format!("solidcraft-parts-convert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("THRUSTERS")).unwrap();
        let step = dir.join("THRUSTERS/BR-100545.step");
        let mut maker = Session::default();
        maker.execute("solid.box", &json!({"length": 40, "width": 20, "height": 10})).unwrap();
        maker.execute("file.export", &json!({"path": step.to_str().unwrap()})).unwrap();
        let mut s = Session::default();
        let before = s.doc.to_json();
        let params = json!({"path": step.to_str().unwrap(), "name": "T200 thruster", "part_number": "BR-100545", "description": "T200 thruster", "stand_up": true});
        let r = s.execute("library.parts.convert", &params).unwrap();
        assert_eq!(s.doc.to_json(), before, "the open design is never touched");
        let file = dir.join("THRUSTERS/BR-100545.solidcraft");
        assert_eq!(r["file"], json!(file.to_str().unwrap()));
        assert_eq!((r["bodies"].as_u64(), r["stood_up"].as_bool()), (Some(1), Some(false)), "SolidCraft's own STEP is Z up: {r}");
        let bytes = std::fs::read(&file).unwrap();
        assert!(solidcraft_io::read_design_thumbnail(&bytes).is_some(), "the part's picture is in the file");
        let doc = solidcraft_io::read_design(&bytes).unwrap();
        assert_eq!(doc.name, "T200 thruster");
        assert_eq!(
            (doc.metadata.0.get("part_number"), doc.metadata.0.get("description")),
            (Some(&json!("BR-100545")), Some(&json!("T200 thruster")))
        );
        // The library lists the design in place of its STEP, and it links.
        let found = s.execute("library.parts.search", &json!({"folder": dir.to_str().unwrap()})).unwrap();
        assert_eq!(
            (found["total"].as_u64(), &found["parts"][0]["kind"], &found["parts"][0]["part_number"]),
            (Some(1), &json!("design"), &json!("BR-100545"))
        );
        s.execute("component.insert_design", &json!({"path": file.to_str().unwrap()})).unwrap();
        assert_eq!(s.doc.links.len(), 1);
        let again = s.execute("library.parts.convert", &params);
        assert!(again.unwrap_err().to_string().contains("already exists"), "converting again needs overwrite");
        s.execute("library.parts.convert", &json!({"path": step.to_str().unwrap(), "overwrite": true})).unwrap();
        for bad in [json!({}), json!({"path": dir.join("notes.pdf").to_str().unwrap()}), json!({"path": step.to_str().unwrap(), "stand_up": "yes"})] {
            assert!(s.execute("library.parts.convert", &bad).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
