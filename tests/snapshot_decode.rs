mod common;
use bc_mdc_converter::mdc::snapshot::decode_snap_method;
use std::io::Read;

fn mdc_bytes() -> Vec<u8> {
    let f = std::fs::File::open(common::fixtures_dir().join("snapshot-page-exception.zip")).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut e = z.by_name("228.mdc").unwrap();
    let mut b = Vec::new();
    e.read_to_end(&mut b).unwrap();
    b
}

fn find<'a>(vars: &'a [bc_mdc_converter::mdc::snapshot::SnapVariable], name: &str) -> &'a bc_mdc_converter::mdc::snapshot::SnapVariable {
    vars.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no variable {name}"))
}

#[test]
fn decodes_line_contexts_variables_and_exception() {
    let m = decode_snap_method(&mdc_bytes()).unwrap();
    assert_eq!(m.method_name, "OnOpenPage");
    assert_eq!((m.object_type_id, m.object_id), (Some(8), Some(50152)));
    assert_eq!(m.line_contexts.len(), 2);
    let lc0 = &m.line_contexts[0];
    assert_eq!((lc0.index, lc0.line_number), (0, 0));
    assert_eq!(lc0.span.as_ref().unwrap().from, (34, 8));
    assert_eq!(find(&lc0.variables, "PageLocalText").value, "''");
    assert_eq!(find(&lc0.variables, "PageLocalText").type_name, "Text[50]");
    let lc1 = &m.line_contexts[1];
    assert_eq!(lc1.span.as_ref().unwrap().from, (35, 8));
    assert_eq!(find(&lc1.variables, "PageLocalText").value, "'page trigger zoo'");
    let globals = find(&lc1.variables, "<Globals>");
    assert!(globals.has_children);
    assert!(globals.children.iter().any(|c| c.name == "Rec"));
    let e = m.exception.as_ref().unwrap();
    assert_eq!(e.message, "page OnOpenPage break");
    assert_eq!(e.line_number, 1);
    assert_eq!(e.span.as_ref().unwrap().from, (35, 8));
    assert_eq!(m.frames[0].object_name, "Trigger Demo Page");
}

#[test]
fn malformed_bytes_are_an_error_not_a_panic() {
    assert!(decode_snap_method(&[]).is_err());
    assert!(decode_snap_method(&[0xff, 0xff, 0xff, 0x7f, 1, 2, 3]).is_err());
    let mut b = mdc_bytes();
    b.truncate(40);
    assert!(decode_snap_method(&b).is_err());
}
