#![allow(dead_code)] // shared helpers; each tests/*.rs compiles its own copy, so subsets are unused per binary

use std::path::PathBuf;

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}
pub fn load_zip_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(format!("{name}.mdc.zip"))).expect("read zip fixture")
}
pub fn load_reference(name: &str) -> serde_json::Value {
    let s = std::fs::read_to_string(fixtures_dir().join(format!("{name}.reference.alcpuprofile")))
        .expect("read reference fixture");
    serde_json::from_str(&s).expect("parse reference json")
}
pub fn load_reference_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(format!("{name}.reference.alcpuprofile")))
        .expect("read reference fixture bytes")
}
