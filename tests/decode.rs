mod common;
use bc_mdc_converter::mdc::decode::decode_snapshot;
use bc_mdc_converter::mdc::schema::{decode_method_context, decode_version};
use bc_mdc_converter::zip::read_zip_entries;
use common::load_zip_bytes;

#[test]
fn reads_all_entries_and_first_mdc_bytes() {
    let bytes = load_zip_bytes("instr");
    let entries = read_zip_entries(&bytes).expect("read zip");
    // .mdc count == 11285 (the .al files and `version` are separate)
    let mdc = entries.iter().filter(|e| e.name.to_ascii_lowercase().ends_with(".mdc")).count();
    assert_eq!(mdc, 11285, "mdc entry count");
    assert!(entries.iter().any(|e| e.name == "version"), "version entry present");
    let zero = entries.iter().find(|e| e.name == "0.mdc").expect("0.mdc");
    // FlatBuffers root uoffset at offset 0 = 0x1c, no file_identifier (.mdc schema spec §2)
    assert_eq!(&zero.data[0..4], &[0x1c, 0x00, 0x00, 0x00]);
}

#[test]
fn decodes_first_mdc_and_version() {
    let bytes = load_zip_bytes("instr");
    let entries = read_zip_entries(&bytes).unwrap();
    let zero = &entries.iter().find(|e| e.name == "0.mdc").unwrap().data;
    let mc = decode_method_context(zero).unwrap();
    assert_eq!(mc.method_name, "GetIsLegacyActionBarEnabled");
    let id = mc.id.expect("id");
    assert_eq!(id.number, 2000000006);
    assert_eq!(id.object_type.name(), "CodeUnit");
    let (start, end) = mc.execution_span.expect("span");
    assert_eq!(start, 63918782549184796);
    assert_eq!(end, 63918782549194949); // delta 10153
    let f0 = &mc.frames[0];
    assert_eq!(f0.object_name, "System Action Triggers");
    // ⚠️ to-before-from trap: `from` must be 278:7, NOT the `to` value 278:8
    let span = f0.statement_span.expect("span");
    assert_eq!((span.from.line, span.from.column), (278, 7));
    assert_eq!((span.to.line, span.to.column), (278, 8));
    assert!(mc.is_builtin);
    assert!(!mc.is_incomplete);

    let ver = &entries.iter().find(|e| e.name == "version").unwrap().data;
    assert_eq!(decode_version(ver), (3, 0, 0, 0)); // major default 3 applies
}

#[test]
fn decode_snapshot_orders_and_gates() {
    let bytes = load_zip_bytes("instr");
    let snap = decode_snapshot(&bytes).expect("decode");
    assert_eq!(snap.contexts.len(), 11285);
    // ordinal 0 is the numerically-first .mdc:
    assert_eq!(snap.contexts[0].method_name, "GetIsLegacyActionBarEnabled");
    assert_eq!(snap.contexts[0].execution_span.unwrap().0, 63918782549184796);
    // ordinal 1 = "1.mdc": object 9175 CodeUnit, has a caller frame (frames len 2) (.mdc schema spec §7)
    assert_eq!(snap.contexts[1].id.unwrap().number, 9175);
    assert_eq!(snap.contexts[1].frames.len(), 2);
}

#[test]
fn decode_snapshot_is_numeric_not_lexicographic() {
    let bytes = load_zip_bytes("instr");
    let entries = read_zip_entries(&bytes).unwrap();
    let d2 = &entries.iter().find(|e| e.name == "2.mdc").expect("2.mdc").data;
    let d10 = &entries.iter().find(|e| e.name == "10.mdc").expect("10.mdc").data;
    let mc2 = decode_method_context(d2).unwrap();
    let mc10 = decode_method_context(d10).unwrap();
    // The test is only meaningful if 2.mdc and 10.mdc actually decode to different
    // contexts. (id.number, method_name) is the discriminating field here.
    assert_ne!(
        (mc2.id.map(|i| i.number), &mc2.method_name),
        (mc10.id.map(|i| i.number), &mc10.method_name)
    );

    let snap = decode_snapshot(&bytes).unwrap();
    // Under a *numeric* sort, index 2 is "2.mdc". Under a lexicographic string sort it
    // would be "10.mdc" (since "10" < "2" as strings). So this assertion passes only for
    // numeric ordering and fails for lexicographic.
    assert_eq!(snap.contexts[2].id.map(|i| i.number), mc2.id.map(|i| i.number));
    assert_eq!(snap.contexts[2].method_name, mc2.method_name);
    assert_ne!(snap.contexts[2].method_name, mc10.method_name);
}

#[test]
fn zip_entries_carry_optional_iso_mtime() {
    let bytes = load_zip_bytes("instr");
    let entries = read_zip_entries(&bytes).unwrap();
    for e in &entries {
        if let Some(m) = &e.mtime_iso {
            let b = m.as_bytes();
            assert_eq!(m.len(), 19, "bad mtime length: {m}");
            assert!(b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':',
                    "bad mtime format: {m}");
        }
    }
}

#[test]
fn snapshot_carries_platform_version_and_optional_mtime() {
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    assert_eq!(snap.platform_version, (3, 0, 0, 0));
    // earliest_mtime_iso is Option — shape only (fixtures may or may not carry timestamps)
    if let Some(m) = &snap.earliest_mtime_iso {
        assert_eq!(m.len(), 19, "bad earliest mtime: {m}");
    }
}
