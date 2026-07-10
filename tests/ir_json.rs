mod common;
use bc_mdc_converter::{convert, EmitOptions, OutputFormat};
use common::load_zip_bytes;
use serde_json::Value;

const FIXTURES: &[(&str, u64)] = &[
    ("instr", 11285),
    ("tiny", 1639),
    ("mixed-objects", 7977),
    ("with-exception", 1477),
    ("long-report", 1887),
];

fn emit(name: &str) -> Value {
    let bytes = load_zip_bytes(name);
    let out = convert(&bytes, OutputFormat::IrJson, &EmitOptions::default()).unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn ir_json_valid_on_all_fixtures() {
    for &(name, count) in FIXTURES {
        let v = emit(name);
        assert_eq!(v["schemaVersion"], 1, "{name}");
        assert_eq!(v["generator"]["name"], "bc-mdc-converter", "{name}");
        assert_eq!(v["capture"]["invocationCount"].as_u64().unwrap(), count, "{name}");
        assert_eq!(v["capture"]["ticksPerMs"], 10000, "{name}");
        assert_eq!(v["capture"]["platformVersion"], "3.0.0.0", "{name}");
        assert!(v["capture"]["t0Ticks"].is_string(), "{name}: t0Ticks must be a string");
        let invs = v["invocations"].as_array().unwrap();
        assert_eq!(invs.len() as u64, count, "{name}");
        let napps = v["apps"].as_array().unwrap().len() as u64;
        assert!(napps > 0, "{name}: expected at least one app");
        for (i, inv) in invs.iter().enumerate() {
            assert_eq!(inv["index"].as_u64().unwrap(), i as u64, "{name}[{i}] index==position");
            // JS-safe rebased magnitudes (spec §3.5)
            for k in ["startTicks", "endTicks", "clampedEndTicks"] {
                if let Some(t) = inv[k].as_i64() {
                    assert!((0..(1_i64 << 53)).contains(&t), "{name}[{i}].{k}={t}");
                }
            }
            let self_ticks = inv["selfTicks"].as_i64().unwrap();
            assert!(self_ticks >= 0, "{name}[{i}] selfTicks");
            if let Some(p) = inv["temporalParentIx"].as_u64() {
                assert!(p < i as u64, "{name}[{i}] temporalParentIx {p} !< index");
            }
            if let Some(a) = inv["appIx"].as_u64() {
                assert!(a < napps, "{name}[{i}] appIx out of range");
            }
            if inv["inSweep"] == false {
                assert_eq!(self_ticks, 0, "{name}[{i}] not-in-sweep selfTicks must be 0");
                assert!(inv["temporalParentIx"].is_null(), "{name}[{i}] not-in-sweep parent must be null");
            }
            // v1 keys always present; nullable fields are explicit null (spec §3.7)
            for k in ["objectType", "objectId", "objectName", "method", "appIx",
                      "startTicks", "endTicks", "clampedEndTicks", "inSweep", "selfTicks",
                      "temporalParentIx", "v8AggregationParentIx", "isBuiltin", "isIncomplete",
                      "calledLine", "callerLine", "lines", "exception"] {
                assert!(inv.get(k).is_some(), "{name}[{i}] missing key {k}");
            }
        }
    }
}

#[test]
fn ir_json_self_ticks_conservation() {
    // Σ selfTicks == length of the union of in-sweep effective intervals: the boundary
    // sweep partitions covered time exactly among leaves (spec §7). Integer-exact.
    for &(name, _) in FIXTURES {
        let v = emit(name);
        let invs = v["invocations"].as_array().unwrap();
        let mut intervals: Vec<(i64, i64)> = invs.iter()
            .filter(|inv| inv["inSweep"] == true)
            .map(|inv| {
                let s = inv["startTicks"].as_i64().unwrap();
                let e = inv["clampedEndTicks"].as_i64()
                    .unwrap_or_else(|| inv["endTicks"].as_i64().unwrap());
                (s, e)
            })
            .collect();
        intervals.sort();
        let (mut covered, mut cur) = (0i64, None::<(i64, i64)>);
        for (s, e) in intervals {
            match cur {
                Some((cs, ce)) if s <= ce => cur = Some((cs, ce.max(e))),
                Some((cs, ce)) => { covered += ce - cs; cur = Some((s, e)); }
                None => cur = Some((s, e)),
            }
        }
        if let Some((cs, ce)) = cur { covered += ce - cs; }
        let sum_self: i64 = invs.iter().map(|inv| inv["selfTicks"].as_i64().unwrap()).sum();
        assert_eq!(sum_self, covered, "{name}: Σ selfTicks != union coverage");
    }
}

#[test]
fn ir_json_exceptions_on_with_exception_fixture() {
    let v = emit("with-exception");
    assert_eq!(v["capture"]["exceptionCount"], 7); // fixtures/manifest.md
    let invs = v["invocations"].as_array().unwrap();
    let n = invs.iter().filter(|i| !i["exception"].is_null()).count();
    assert_eq!(n, 7);
    let msg = invs.iter()
        .find_map(|i| i["exception"]["message"].as_str().map(String::from))
        .unwrap();
    assert!(msg.contains("not present in the dictionary"), "unexpected message: {msg}");
}

#[test]
fn ir_json_rebasing_and_app_dedup() {
    let v = emit("instr");
    // rebasing to t0: at least one invocation starts at exactly 0
    assert!(v["invocations"].as_array().unwrap().iter()
        .any(|i| i["startTicks"].as_i64() == Some(0)), "no invocation at t0");
    // apps deduped by id-else-name key
    let apps = v["apps"].as_array().unwrap();
    let mut keys: Vec<&str> = apps.iter().map(|a| {
        let id = a["id"].as_str().unwrap();
        if !id.is_empty() { id } else { a["name"].as_str().unwrap() }
    }).collect();
    let before = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), before, "duplicate app keys");
    // objectType uses the V8 spelling (spec §3.6)
    assert!(v["invocations"].as_array().unwrap().iter()
        .any(|i| i["objectType"] == "CodeUnit"), "expected V8 spelling 'CodeUnit'");
}

#[test]
fn ir_json_synthetic_edge_rows() {
    // Spec §7 unit cases: null-span row, clamped incomplete row, exception row (raw wire
    // line, no +1), object-less row. Emitter called directly on a hand-built Profile.
    use bc_mdc_converter::emit::ir_json::IrJsonEmitter;
    use bc_mdc_converter::ir::{Exception, FrameInfo, Invocation, Profile, Span};
    use bc_mdc_converter::mdc::types::{ApplicationObjectId, ObjectType};
    use bc_mdc_converter::Emitter;

    let id = ApplicationObjectId { number: 80, object_type: ObjectType::CodeUnit };
    let frame = |m: &str| Some(FrameInfo {
        method_name: m.into(), id: Some(id), object_name: "O".into(), statement_span: None,
    });
    let base = |ix: u32| Invocation {
        index: ix, object: Some(id), this_frame: frame("M"), app: None, span: None,
        is_builtin: false, is_incomplete: false, parent_ix: None, child_ixs: vec![],
        caller_line: None, called_line: None, lines: vec![], exception: None, sql: vec![],
    };

    let mut parent = base(0);
    parent.span = Some(Span { start_ticks: 100, end_ticks: 1_000_100 });
    let mut clamped = base(1); // incomplete, raw end past parent -> clamped to parent end
    clamped.span = Some(Span { start_ticks: 100, end_ticks: 3_000_100 });
    clamped.is_incomplete = true;
    clamped.parent_ix = Some(0);
    let mut thrown = base(2); // exception with raw wire line 5
    thrown.span = Some(Span { start_ticks: 200, end_ticks: 300 });
    thrown.parent_ix = Some(0);
    thrown.exception = Some(Exception { message: "boom".into(), line: 5 });
    let spanless = base(3);   // no span -> null ticks, not in sweep
    let mut objectless = base(4); // span but no object -> no sweep events either
    objectless.object = None;
    objectless.span = Some(Span { start_ticks: 400, end_ticks: 500 });

    let profile = Profile {
        start_time_ticks: 100, end_time_ticks: 1_000_100,
        invocations: vec![parent, clamped, thrown, spanless, objectless],
        apps: vec![], capture: Default::default(),
    };
    let out = IrJsonEmitter.emit(&profile, &EmitOptions::default()).unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let invs = v["invocations"].as_array().unwrap();

    // t0 = 100 -> parent rebased to 0
    assert_eq!(invs[0]["startTicks"], 0);
    assert_eq!(invs[0]["clampedEndTicks"], Value::Null); // raw end used -> null
    // clamped: raw end 3_000_100 -> effective 1_000_100; both rebased
    assert_eq!(invs[1]["endTicks"], 3_000_000);
    assert_eq!(invs[1]["clampedEndTicks"], 1_000_000);
    assert_eq!(invs[1]["inSweep"], true);
    // exception: raw wire line, NO +1 (spec §3.5)
    assert_eq!(invs[2]["exception"]["line"], 5);
    assert_eq!(invs[2]["exception"]["message"], "boom");
    // span-less: explicit nulls, out of sweep
    assert_eq!(invs[3]["startTicks"], Value::Null);
    assert_eq!(invs[3]["inSweep"], false);
    assert_eq!(invs[3]["selfTicks"], 0);
    // object-less: null object triple, out of sweep despite having a span
    assert_eq!(invs[4]["objectType"], Value::Null);
    assert_eq!(invs[4]["inSweep"], false);
    // capture with Default metadata
    assert_eq!(v["capture"]["platformVersion"], "0.0.0.0");
    assert_eq!(v["capture"]["approxWallClockStart"], Value::Null);
    assert_eq!(v["capture"]["exceptionCount"], 1);
}

#[test]
fn ir_json_populated_wall_clock_start() {
    // All committed fixtures carry a real zip DOS mod-time, so approxWallClockStart
    // is a populated ISO string (decode -> capture -> wire plumbing for the non-null
    // path; the synthetic edge test covers the null path). Shape, not exact value —
    // the value is a property of the fixture file's mtime.
    let v = emit("tiny");
    let s = v["capture"]["approxWallClockStart"].as_str()
        .expect("approxWallClockStart should be a populated ISO string");
    assert_eq!(s.len(), 19, "expected YYYY-MM-DDTHH:MM:SS, got {s}");
    let b = s.as_bytes();
    assert!(b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':',
            "bad ISO shape: {s}");
}

#[test]
fn ir_json_empty_capture() {
    // Spec §6: an empty capture emits a valid document — t0Ticks "0", empty apps
    // and invocations, zeroed counts.
    use bc_mdc_converter::emit::ir_json::IrJsonEmitter;
    use bc_mdc_converter::ir::Profile;
    use bc_mdc_converter::Emitter;

    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 0,
        invocations: vec![], apps: vec![], capture: Default::default(),
    };
    let out = IrJsonEmitter.emit(&profile, &EmitOptions::default()).unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["schemaVersion"], 1);
    assert_eq!(v["capture"]["t0Ticks"], "0");
    assert_eq!(v["capture"]["invocationCount"], 0);
    assert_eq!(v["capture"]["incompleteCount"], 0);
    assert_eq!(v["capture"]["exceptionCount"], 0);
    assert_eq!(v["apps"].as_array().unwrap().len(), 0);
    assert_eq!(v["invocations"].as_array().unwrap().len(), 0);
}
