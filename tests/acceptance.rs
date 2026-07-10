mod common;
use bc_mdc_converter::{convert, EmitOptions, OutputFormat};
use common::load_reference;

#[test]
fn reference_fixture_loads_with_706_nodes() {
    let r = load_reference("instr");
    let nodes = r["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 706);
    let samples = r["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 706);
    // hit-count conservation in the reference itself:
    let sum: i64 = nodes.iter().map(|n| n["hitCount"].as_i64().unwrap_or(0)).sum();
    assert_eq!(sum, 11285);
}

#[test]
fn emits_node_1_matching_reference() {
    let bytes = common::load_zip_bytes("instr");
    let out = convert(&bytes, OutputFormat::V8, &EmitOptions::default()).unwrap();
    let got: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let reference = common::load_reference("instr");
    // node 1
    let g1 = &got["nodes"][0];
    let r1 = &reference["nodes"][0];
    assert_eq!(g1["id"], r1["id"]);
    assert_eq!(g1["callFrame"]["functionName"], r1["callFrame"]["functionName"]);
    assert_eq!(g1["callFrame"]["scriptId"], r1["callFrame"]["scriptId"]); // CodeUnit_2000000006
    assert_eq!(g1["callFrame"]["url"], r1["callFrame"]["url"]);           // al-preview://allang/... Codeunit ...
    assert_eq!(g1["callFrame"]["lineNumber"], r1["callFrame"]["lineNumber"]); // 278
    assert_eq!(g1["hitCount"], r1["hitCount"]);
    assert_eq!(g1["children"], r1["children"]);                          // [2]
    assert_eq!(g1["positionTicks"], r1["positionTicks"]);                // line 278 col 7 ticks 1 execTime 7949
    assert_eq!(g1["frameIdentifier"], r1["frameIdentifier"]);            // 602714591
    assert_eq!(got["startTime"], reference["startTime"]);
    assert_eq!(got["endTime"], reference["endTime"]);
    // isBuiltinCodeUnitCall present only when true; node 1 is builtin => present
    assert_eq!(g1["isBuiltinCodeUnitCall"], serde_json::json!(true));
}

fn assert_structural_match(name: &str, expect_nodes: usize) {
    let bytes = common::load_zip_bytes(name);
    let out = convert(&bytes, OutputFormat::V8, &EmitOptions::default()).unwrap();
    let got: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let want = common::load_reference(name);

    // top-level (algorithm spec §10.1.1)
    assert_eq!(got["startTime"], want["startTime"], "{name} startTime");
    assert_eq!(got["endTime"], want["endTime"], "{name} endTime");
    for key in ["nodes","samples","timeDeltas","sampleExecutionTimes"] {
        assert_eq!(got[key].as_array().unwrap().len(), expect_nodes, "{name} {key} len");
    }
    assert_eq!(got["samples"], want["samples"], "{name} samples");
    assert_eq!(got["timeDeltas"], want["timeDeltas"], "{name} timeDeltas");
    assert_eq!(got["sampleExecutionTimes"], want["sampleExecutionTimes"], "{name} sampleExecutionTimes");

    // per-node full equality (id-keyed) — nodes already id-sorted on both sides
    let gn = got["nodes"].as_array().unwrap();
    let wn = want["nodes"].as_array().unwrap();
    assert_eq!(gn.len(), wn.len(), "{name} node count");
    for (g, w) in gn.iter().zip(wn.iter()) {
        assert_eq!(g, w, "{name} node id {}", w["id"]);
    }
    // conservation
    let sum: i64 = gn.iter().map(|n| n["hitCount"].as_i64().unwrap_or(0)).sum();
    let wsum: i64 = wn.iter().map(|n| n["hitCount"].as_i64().unwrap_or(0)).sum();
    assert_eq!(sum, wsum, "{name} hitCount conservation");
}

#[test] fn acceptance_instr()          { assert_structural_match("instr", 706); }
#[test] fn acceptance_tiny()           { assert_structural_match("tiny", 492); }
#[test] fn acceptance_mixed_objects()  { assert_structural_match("mixed-objects", 648); }
#[test] fn acceptance_with_exception() { assert_structural_match("with-exception", 470); }
#[test] fn acceptance_long_report()    { assert_structural_match("long-report", 511); }

#[test]
fn byte_identical_all_fixtures() {
    for name in ["instr", "tiny", "mixed-objects", "with-exception", "long-report"] {
        let bytes = common::load_zip_bytes(name);
        let out = convert(&bytes, OutputFormat::V8, &EmitOptions::default()).unwrap();
        let reference = common::load_reference_bytes(name);
        assert_eq!(out, reference, "{name}: output bytes must be byte-identical to reference");
    }
}
