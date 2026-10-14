mod common;
use bc_mdc_converter::snapshot_json::{write_snapshot_json, SnapshotLimits};

fn fixture() -> Vec<u8> { std::fs::read(common::fixtures_dir().join("snapshot-page-exception.zip")).unwrap() }
fn run(zip: &[u8], limits: &SnapshotLimits) -> bc_mdc_converter::Result<serde_json::Value> {
    let mut out = Vec::new();
    write_snapshot_json(zip, &mut out, limits)?;
    Ok(serde_json::from_slice(&out).unwrap())
}

#[test]
fn emits_the_contract_for_the_cronus29_capture() {
    let j = run(&fixture(), &SnapshotLimits::default()).unwrap();
    assert_eq!(j["format"], "bc-mdc-snapshot");
    assert_eq!(j["schemaVersion"], 1);
    let m = &j["methods"][0];
    assert_eq!(m["entry"], "228.mdc");
    assert_eq!(m["object"]["type"], "Page");
    assert_eq!(m["object"]["typeId"], 8);
    assert_eq!(m["object"]["id"], 50152);
    assert_eq!(m["object"]["name"], "Trigger Demo Page");
    assert_eq!(m["lineContexts"][1]["span"]["from"]["line"], 35);
    assert_eq!(m["lineContexts"][0]["timestamp"], "0");
    let v1 = m["lineContexts"][1]["variables"].as_array().unwrap();
    assert!(v1.iter().any(|v| v["name"] == "PageLocalText" && v["value"] == "'page trigger zoo'"));
    assert_eq!(m["exception"]["message"], "page OnOpenPage break");
    assert_eq!(m["exception"]["lineNumber"], 1);
    assert_eq!(j["sources"][0], serde_json::json!({ "entry": "Page%50152.al", "objectType": "Page", "objectId": 50152 }));
}

#[test]
fn output_over_the_limit_is_an_error() {
    let limits = SnapshotLimits { max_output_bytes: 200, ..SnapshotLimits::default() };
    assert!(run(&fixture(), &limits).is_err());
}

#[test]
fn entry_over_the_limit_is_an_error() {
    let limits = SnapshotLimits { max_entry_bytes: 100, ..SnapshotLimits::default() };
    assert!(run(&fixture(), &limits).is_err());
}

#[test]
fn not_a_zip_is_an_error() {
    assert!(run(b"definitely not a zip", &SnapshotLimits::default()).is_err());
}

#[test]
fn non_ascii_values_round_trip() {
    let mut out = Vec::new();
    bc_mdc_converter::snapshot_json::write_str_for_test(&mut out, "Kontorcentralen A/S æøå \"q\"\n").unwrap();
    assert_eq!(serde_json::from_slice::<String>(&out).unwrap(), "Kontorcentralen A/S æøå \"q\"\n");
}
