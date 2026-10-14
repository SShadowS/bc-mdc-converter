mod common;
use std::process::Command;

fn bin() -> &'static str { env!("CARGO_BIN_EXE_bc-mdc-converter") }

#[test]
fn cli_converts_instr_to_v8() {
    let dir = std::env::temp_dir();
    let out = dir.join("bcmdc_cli_test.alcpuprofile");
    let in_path = common::fixtures_dir().join("instr.mdc.zip");
    let status = Command::new(bin())
        .arg(&in_path).arg(&out)
        .status().unwrap();
    assert!(status.success());
    let produced = std::fs::read(&out).unwrap();
    let got: serde_json::Value = serde_json::from_slice(&produced).unwrap();
    assert_eq!(got["nodes"].as_array().unwrap().len(), 706);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn cli_rejects_bad_args() {
    let status = Command::new(bin()).arg("only-one").status().unwrap();
    assert!(!status.success());
}

#[test]
fn cli_converts_tiny_to_firefox() {
    let out = std::env::temp_dir().join("bcmdc_ff_test.json");
    let in_path = common::fixtures_dir().join("tiny.mdc.zip");
    let status = Command::new(bin())
        .arg(&in_path).arg(&out).arg("--format").arg("firefox")
        .status().unwrap();
    assert!(status.success());
    let produced = std::fs::read(&out).unwrap();
    let p: serde_json::Value = serde_json::from_slice(&produced).unwrap();
    assert_eq!(p["meta"]["preprocessedProfileVersion"], 54);
    assert_eq!(p["threads"][0]["samples"]["weightType"], "tracing-ms");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn cli_converts_tiny_to_ir_json() {
    let out = std::env::temp_dir().join("bcmdc_ir_test.ir.json");
    let in_path = common::fixtures_dir().join("tiny.mdc.zip");
    let status = Command::new(bin())
        .arg(&in_path).arg(&out).arg("--format").arg("ir-json")
        .status().unwrap();
    assert!(status.success());
    let p: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(p["schemaVersion"], 1);
    assert_eq!(p["capture"]["invocationCount"], 1639);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn cli_version_and_formats() {
    let v = Command::new(bin()).arg("--version").output().unwrap();
    assert!(v.status.success());
    assert_eq!(String::from_utf8(v.stdout).unwrap().trim(), format!("bc-mdc-converter {}", env!("CARGO_PKG_VERSION")));
    let f = Command::new(bin()).arg("--formats").output().unwrap();
    assert_eq!(String::from_utf8(f.stdout).unwrap(), "v8\nfirefox\nir-json\nsnapshot-json\n");
}

#[test]
fn cli_snapshot_json_writes_file_and_deletes_partial_on_error() {
    let out = std::env::temp_dir().join("bcmdc_snapshot_test.json");
    let ok = Command::new(bin())
        .arg(common::fixtures_dir().join("snapshot-page-exception.zip")).arg(&out)
        .arg("--format").arg("snapshot-json").status().unwrap();
    assert!(ok.success());
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(j["format"], "bc-mdc-snapshot");
    let bad_in = std::env::temp_dir().join("bcmdc_not_a_zip.zip");
    std::fs::write(&bad_in, b"nope").unwrap();
    let bad = Command::new(bin()).arg(&bad_in).arg(&out).arg("--format").arg("snapshot-json").output().unwrap();
    assert!(!bad.status.success());
    assert!(!out.exists(), "partial output must be deleted");
    let _ = std::fs::remove_file(&bad_in);
}
