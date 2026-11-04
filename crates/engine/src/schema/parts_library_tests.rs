use serde_json::json;

use super::{PartsConvertParams, PartsSearchParams, PartsSearchReply};
use crate::Session;
use crate::schema::tests::{assert_keys, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<PartsSearchParams>(), &["folder", "limit", "query"]);
    assert_keys(input_schema_for::<PartsConvertParams>(), &["description", "name", "organization", "overwrite", "part_number", "path", "revision", "stand_up"]);
}

#[test]
fn replies_match_the_output_schema_and_strict_keeps_todays_errors() {
    let dir = std::env::temp_dir().join(format!("solidcraft-parts-schema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("FITTINGS")).unwrap();
    std::fs::write(dir.join("FITTINGS/Penetrator.step"), b"x").unwrap();
    let schema = output_schema_for::<PartsSearchReply>();
    let schema = schema.as_value();
    let mut s = Session { param_check: ParamCheck::Warn, ..Default::default() };
    let r = s.execute("library.parts.search", &json!({"folder": dir.to_str().unwrap(), "query": "penetrator", "limit": 5})).unwrap();
    assert!(r.get("param_warnings").is_none(), "{r}");
    assert!(conforms(schema, schema, &r), "{r} doesn't match {schema}");
    s.param_check = ParamCheck::Strict;
    for (params, message) in [
        (json!({"folder": ""}), "must be a folder path"),
        (json!({"folder": dir.to_str().unwrap(), "limit": 0}), "1 or more"),
        (json!({"folder": dir.to_str().unwrap(), "query": 3}), "must be text"),
    ] {
        let e = s.execute("library.parts.search", &params).unwrap_err().to_string();
        assert!(e.contains(message), "{params}: {e}");
    }
    let e = s.execute("library.parts.search", &json!({"folder": dir.to_str().unwrap(), "colour": "red"})).unwrap_err().to_string();
    assert!(e.contains("colour"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
