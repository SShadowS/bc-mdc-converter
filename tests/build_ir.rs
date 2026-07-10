mod common;
use bc_mdc_converter::mdc::decode::decode_snapshot;
use bc_mdc_converter::build_ir::build_profile;
use common::load_zip_bytes;

#[test]
fn ir_conserves_hits_and_links_root_child() {
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    assert_eq!(profile.invocations.len(), 11285);
    // hit-count conservation as an IR property: every invocation with a valid object
    // starts as one hit; Σ over raw line hits is a different quantity, so assert the
    // frame-count identity that Phase 4 will preserve:
    let framed = profile.invocations.iter().filter(|i| i.object.is_some()).count();
    assert_eq!(framed, 11285); // all instr contexts have a valid id
    // profile time sourcing (algorithm spec §7.1):
    assert_eq!(profile.start_time_ticks, 63918782549184796);
    assert_eq!(profile.end_time_ticks, 63918782658043051);
    // invocation 0 is a root (single frame => no caller line):
    assert!(profile.invocations[0].caller_line.is_none());
    assert!(profile.invocations[0].parent_ix.is_none());
    // invocation 1's parent is invocation 0 (.mdc schema spec §7 caller/called shape):
    assert_eq!(profile.invocations[1].parent_ix, Some(0));
    // invocation 0 lists 1 as a child:
    assert!(profile.invocations[0].child_ixs.contains(&1));
}
