//! Parts libraries (docs/parts-library.md): `library.parts.search` lists and searches the parts
//! in a library folder. The Parts Library window (`ui.start` of this command) searches the same
//! list and inserts a part with `component.insert_design` (a design, linked) or
//! `file.insert_step` (a CAD file, copied in).

use serde_json::Value;
use solidcraft_io::parts_library::{self, Kind};

use super::CommandSpec;
use crate::params::bad;
use crate::schema::parts_library::{LibraryPart, PartsSearchParams, PartsSearchReply};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const SEARCH_CONTRACT: TypedContract<PartsSearchParams> = TypedContract::new();

/// Parts returned when the call names no limit, and the most it may name.
const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 5000;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new("library.parts.search", "Parts Library", crate::checked!("library.parts.search", search))
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
    )];

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
}
