use serde_json::json;

use super::{
    VersionsCheckInParams, VersionsCopyParams, VersionsFolderParams, VersionsHistoryParams, VersionsNameParams, VersionsRestoreParams,
    VersionsStatusReply,
};
use crate::Session;
use crate::schema::tests::{assert_keys, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<VersionsFolderParams>(), &["folder"]);
    assert_keys(input_schema_for::<VersionsCheckInParams>(), &["author", "folder", "message", "paths", "release"]);
    assert_keys(input_schema_for::<VersionsHistoryParams>(), &["folder", "limit", "path"]);
    assert_keys(input_schema_for::<VersionsRestoreParams>(), &["commit", "folder", "path"]);
    assert_keys(input_schema_for::<VersionsCopyParams>(), &["commit", "folder", "path", "to"]);
    assert_keys(input_schema_for::<VersionsNameParams>(), &["author", "commit", "folder", "path", "revision", "version"]);
}

#[test]
fn replies_match_the_output_schema_and_strict_keeps_todays_errors() {
    let dir = std::env::temp_dir().join(format!("solidcraft-versions-schema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Part.step"), b"x").unwrap();
    let f = dir.to_str().unwrap();
    let schema = output_schema_for::<VersionsStatusReply>();
    let schema = schema.as_value();
    let mut s = Session { param_check: ParamCheck::Warn, ..Default::default() };
    let r = s.execute("versions.start", &json!({"folder": f})).unwrap();
    assert!(r.get("param_warnings").is_none(), "{r}");
    assert!(conforms(schema, schema, &r), "{r} doesn't match {schema}");
    s.param_check = ParamCheck::Strict;
    for (id, params, message) in [
        ("versions.status", json!({"folder": ""}), "must be a folder path"),
        ("versions.history", json!({"folder": f, "limit": 0}), "1 or more"),
        ("versions.check_in", json!({"folder": f, "message": "x", "paths": []}), "list of files"),
        ("versions.check_in", json!({"folder": f, "message": "x", "paths": [3]}), "list of files"),
    ] {
        let e = s.execute(id, &params).unwrap_err().to_string();
        assert!(e.contains(message), "{params}: {e}");
    }
    let e = s.execute("versions.status", &json!({"folder": f, "colour": "red"})).unwrap_err().to_string();
    assert!(e.contains("colour"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
