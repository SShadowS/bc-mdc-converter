mod common;
use bc_mdc_converter::mdc::decode::decode_snapshot;
use bc_mdc_converter::build_ir::build_profile;
use bc_mdc_converter::emit::firefox::{tick_to_ms, wire_line_to_display, profile_t0};
use common::load_zip_bytes;

#[test]
fn tick_scale_is_100ns_confirmed_by_capture_span() {
    // instr capture is 10.886 s (fixtures/manifest.md). endTime-startTime ticks / 10000 == ms.
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let t0 = profile_t0(&profile);
    assert_eq!(t0, profile.start_time_ticks, "t0 == ordinal-0 start for a single-root capture");
    let span_ms = tick_to_ms(profile.end_time_ticks, t0);
    // 10.886 s -> ~10885.8 ms; allow a small window (exact fixture value is 10885.8255)
    assert!((span_ms - 10885.8).abs() < 1.0, "span ms was {span_ms}, expected ~10885.8");
}

#[test]
fn line_and_ms_helpers() {
    assert_eq!(wire_line_to_display(277), 278);         // Firefox emitter spec §7.3: +1
    assert_eq!(tick_to_ms(10000, 0), 1.0);              // 10000 ticks = 1 ms
    assert_eq!(tick_to_ms(63918782549194949, 63918782549184796), 1.0153); // 10153 ticks
}

#[test]
fn with_exception_fixture_populates_ir_exceptions() {
    let snap = decode_snapshot(&load_zip_bytes("with-exception")).unwrap();
    let profile = build_profile(&snap.contexts);
    let exc: Vec<&bc_mdc_converter::ir::Exception> =
        profile.invocations.iter().filter_map(|i| i.exception.as_ref()).collect();
    // manifest.md: with-exception carries 7 populated ExceptionContext tables
    assert_eq!(exc.len(), 7, "expected 7 exceptions");
    assert!(exc.iter().all(|e| e.message == "The given key was not present in the dictionary."),
            "known platform exception message (manifest.md)");
    // clean-contrast fixtures have zero:
    for clean in ["tiny", "long-report"] {
        let s = decode_snapshot(&load_zip_bytes(clean)).unwrap();
        let p = build_profile(&s.contexts);
        assert_eq!(p.invocations.iter().filter(|i| i.exception.is_some()).count(), 0, "{clean} has no exceptions");
    }
}

use bc_mdc_converter::emit::firefox::build::build_tables;
use bc_mdc_converter::EmitOptions;

#[test]
fn tables_dedupe_and_are_topologically_ordered() {
    let snap = decode_snapshot(&load_zip_bytes("tiny")).unwrap();
    let profile = build_profile(&snap.contexts);
    let t = build_tables(&profile, &EmitOptions::default());
    // one stack node per framed invocation; count matches
    let framed = profile.invocations.iter().filter(|i| i.object.is_some()).count();
    assert_eq!(t.stacks.frame.len(), framed);
    // funcs are deduped: fewer funcs than frames than stacks (loops/repeats collapse)
    assert!(t.funcs.name.len() <= t.frames.func.len());
    assert!(t.frames.func.len() <= t.stacks.frame.len());
    // topological: every prefix is a strictly-lower index or null
    for (i, p) in t.stacks.prefix.iter().enumerate() {
        if let Some(px) = p { assert!((*px as usize) < i, "prefix {px} !< node {i}"); }
    }
    // every frame->func and stack->frame index resolves
    for &f in &t.stacks.frame { assert!((f as usize) < t.frames.func.len()); }
    for &fu in &t.frames.func { assert!((fu as usize) < t.funcs.name.len()); }
    // resources = distinct apps; every funcTable.resource is -1 or valid
    for &r in &t.funcs.resource { assert!(r == -1 || (r as usize) < t.resources.name.len()); }
}

#[test]
fn functable_names_use_firefox_codeunit_casing() {
    // Firefox emitter spec spells the object type "Codeunit" (lowercase u), not the V8/scriptId "CodeUnit".
    // The `instr` fixture has CodeUnit objects, so at least one func name must start with
    // "Codeunit " and NONE may start with "CodeUnit ".
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let t = build_tables(&profile, &EmitOptions::default());
    let names: Vec<&str> = t.funcs.name.iter().map(|&ix| t.interner.strings[ix as usize].as_str()).collect();
    assert!(names.iter().any(|n| n.starts_with("Codeunit ")), "expected a 'Codeunit ' func name");
    assert!(!names.iter().any(|n| n.starts_with("CodeUnit ")), "no func name may start with 'CodeUnit '");
    // Object names must be quoted EXACTLY ONCE: quote_identifier_if_needed already supplies the
    // surrounding quotes, so no func name may contain a doubled `""` (the double-quote bug).
    assert!(!names.iter().any(|n| n.contains("\"\"")), "no func name may contain a doubled quote (double-quoting bug)");
    // instr has object 2000000006 "System Action Triggers" (a space-named object) -> exactly ONE
    // quoted segment `"System Action Triggers"`.
    assert!(names.iter().any(|n| n.contains("\"System Action Triggers\"")),
            "expected a single-quoted 'System Action Triggers' object segment");
}

use bc_mdc_converter::emit::firefox::sweep::build_samples;
use bc_mdc_converter::ir::{Profile, Invocation, Span, FrameInfo};
use bc_mdc_converter::mdc::types::{ApplicationObjectId, ObjectType, SourceSpan, SourcePosition};

fn inv_ic(ix: u32, obj_num: i32, start: i64, end: i64, parent: Option<u32>, children: Vec<u32>, is_incomplete: bool) -> Invocation {
    let id = ApplicationObjectId { number: obj_num, object_type: ObjectType::CodeUnit };
    let sp = SourceSpan { from: SourcePosition { line: 0, column: 0 }, to: SourcePosition { line: 0, column: 0 } };
    Invocation {
        index: ix, object: Some(id),
        this_frame: Some(FrameInfo { method_name: "M".into(), id: Some(id), object_name: "O".into(), statement_span: Some(sp) }),
        app: None, span: Some(Span { start_ticks: start, end_ticks: end }),
        is_builtin: false, is_incomplete,
        parent_ix: parent, child_ixs: children,
        caller_line: None, called_line: Some(bc_mdc_converter::ir::LineDef { id, span: sp }),
        lines: vec![], exception: None, sql: vec![],
    }
}

fn inv(ix: u32, obj_num: i32, start: i64, end: i64, parent: Option<u32>, children: Vec<u32>) -> Invocation {
    inv_ic(ix, obj_num, start, end, parent, children, false)
}

#[test]
fn boundary_sweep_matches_docs07_worked_example() {
    // Firefox emitter spec §8: ticks with t0=0, 10000 ticks = 1 ms
    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 1_300_000,
        invocations: vec![
            inv(0, 80,   0,       1_300_000, None,    vec![1,3]),
            inv(1, 80,   200_000, 900_000,   Some(0), vec![2]),
            inv(2, 37,   300_000, 600_000,   Some(1), vec![]),
            inv(3, 6620, 1_000_000, 1_250_000, Some(0), vec![]),
        ],
        apps: vec![],
        capture: Default::default(),
    };
    // stack node per invocation index (identity for this synthetic case)
    let stack_of = vec![Some(0u32), Some(1), Some(2), Some(3)];
    let s = build_samples(&profile, &stack_of, 0);
    assert_eq!(s.stack, vec![Some(0),Some(1),Some(2),Some(1),Some(0),Some(3),Some(0)]);
    assert_eq!(s.time,   vec![0.0, 20.0, 30.0, 60.0, 90.0, 100.0, 125.0]);
    assert_eq!(s.weight, vec![20.0, 10.0, 30.0, 30.0, 10.0, 25.0, 5.0]);
    assert_eq!(s.weight_type, "tracing-ms");
    // §7.1 conservation: Σ weight == total covered time (root running) == 130 ms
    let total: f64 = s.weight.iter().sum();
    assert!((total - 130.0).abs() < 1e-9, "Σweight was {total}");
}

#[test]
fn sweep_conserves_total_time_on_fixture() {
    let snap = decode_snapshot(&load_zip_bytes("tiny")).unwrap();
    let profile = build_profile(&snap.contexts);
    let t = build_tables(&profile, &EmitOptions::default());
    let t0 = profile_t0(&profile);
    let s = build_samples(&profile, &t.stack_node_of_invocation, t0);
    let total: f64 = s.weight.iter().sum();
    let expected = tick_to_ms(profile.end_time_ticks, t0);
    assert!((total - expected).abs() < 1.0, "Σweight {total} vs covered {expected}");
    assert!(s.time.windows(2).all(|w| w[0] <= w[1]), "time non-decreasing");
    assert!(s.length >= 1);
}

#[test]
fn boundary_sweep_tie_break_shared_boundaries() {
    // Firefox emitter spec §1.3 tie-break: nested spans sharing a START tick (ENTER parent before ENTER
    // child) and adjacent siblings sharing a tick (EXIT before ENTER). t0=0, 10000 ticks = 1ms.
    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 1_000_000,
        invocations: vec![
            inv(0, 80, 0,       1_000_000, None,    vec![1, 2]),          // P
            inv(1, 80, 0,       400_000,   Some(0), vec![]),              // A: shares START 0 with P
            inv(2, 37, 400_000, 1_000_000, Some(0), vec![]),             // B: shares 400_000 w/ A EXIT, END w/ P
        ],
        apps: vec![],
        capture: Default::default(),
    };
    let stack_of = vec![Some(0u32), Some(1), Some(2)];
    let s = build_samples(&profile, &stack_of, 0);
    // A is leaf in [0,40) (ENTER P precedes ENTER A); B is leaf in [40,100) (EXIT A precedes ENTER B).
    assert_eq!(s.stack,  vec![Some(1), Some(2)]);
    assert_eq!(s.time,   vec![0.0, 40.0]);
    assert_eq!(s.weight, vec![40.0, 60.0]);
    let total: f64 = s.weight.iter().sum();
    assert!((total - 100.0).abs() < 1e-9, "Σweight was {total}"); // self(P)=0, self(A)=40, self(B)=60
}

#[test]
fn boundary_sweep_clamps_incomplete_end_to_parent() {
    // Firefox emitter spec §1.3 incomplete clamp: an is_incomplete child whose end runs past its parent's
    // end must be clamped to the parent end. t0=0, 10000 ticks = 1ms.
    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 1_000_000,
        invocations: vec![
            inv(0, 80, 0, 1_000_000, None,    vec![1]),                   // P: 0–100ms
            inv_ic(1, 80, 0, 3_000_000, Some(0), vec![], true),           // C: 0–300ms, incomplete -> clamp to 100ms
        ],
        apps: vec![],
        capture: Default::default(),
    };
    let stack_of = vec![Some(0u32), Some(1), Some(2)];
    let s = build_samples(&profile, &stack_of, 0);
    assert_eq!(s.stack,  vec![Some(1)]);
    assert_eq!(s.time,   vec![0.0]);
    assert_eq!(s.weight, vec![100.0]); // clamped to parent end (100ms), NOT 300ms
    let total: f64 = s.weight.iter().sum();
    assert!((total - 100.0).abs() < 1e-9, "Σweight was {total}");
}

#[test]
fn boundary_sweep_clamps_incomplete_root_to_max_descendant_end() {
    // Firefox emitter spec §1.3 root branch: an is_incomplete ROOT (no usable parent span) with a garbage
    // over-long end is clamped to the max end_ticks over its transitive descendants. t0=0.
    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 9_000_000,
        invocations: vec![
            inv_ic(0, 80, 0, 9_000_000, None, vec![1], true),            // P: garbage 900ms, root, incomplete
            inv(1, 80, 0, 500_000, Some(0), vec![]),                     // C: 0–50ms (max descendant end)
        ],
        apps: vec![],
        capture: Default::default(),
    };
    let stack_of = vec![Some(0u32), Some(1)];
    let s = build_samples(&profile, &stack_of, 0);
    // P clamped to max descendant end 500_000 (50ms), NOT 900ms; C is the only leaf across [0,50).
    assert_eq!(s.stack,  vec![Some(1)]);
    assert_eq!(s.time,   vec![0.0]);
    assert_eq!(s.weight, vec![50.0]);
    let total: f64 = s.weight.iter().sum();
    assert!((total - 50.0).abs() < 1e-9, "Σweight was {total}");
}

use bc_mdc_converter::emit::firefox::markers::build_markers;

#[test]
fn markers_one_bccall_per_invocation_plus_exceptions() {
    let snap = decode_snapshot(&load_zip_bytes("with-exception")).unwrap();
    let profile = build_profile(&snap.contexts);
    let _t0 = profile_t0(&profile); // sanity: callable here, matches brief's setup shape
    // self times (duration - children durations)
    let self_ms: Vec<f64> = profile.invocations.iter().map(|inv| {
        let dur = inv.span.map(|s| s.end_ticks - s.start_ticks).unwrap_or(0);
        let ch: i64 = inv.child_ixs.iter().filter_map(|&c| profile.invocations[c as usize].span)
            .map(|s| s.end_ticks - s.start_ticks).sum();
        (dur - ch) as f64 / 10000.0
    }).collect();
    let mut interner = bc_mdc_converter::emit::firefox::build::Interner::default();
    let m = build_markers(&profile, &mut interner, &self_ms);
    let framed = profile.invocations.iter().filter(|i| i.span.is_some() && i.object.is_some()).count();
    let exc = profile.invocations.iter().filter(|i| i.exception.is_some()).count();
    assert_eq!(exc, 7);
    // BcCall per framed invocation + BcException per throw
    let bccall = m.data.iter().filter(|d| d["type"] == "BcCall").count();
    let bcexc  = m.data.iter().filter(|d| d["type"] == "BcException").count();
    assert_eq!(bccall, framed);
    assert_eq!(bcexc, 7);
    // every INTERVAL marker has both start and end; every marker category non-null
    for i in 0..m.length {
        if m.phase[i] == 1 { assert!(m.start_time[i].is_some() && m.end_time[i].is_some()); }
    }
    // a BcException payload carries the known message
    assert!(m.data.iter().any(|d| d["type"]=="BcException"
        && d["message"]=="The given key was not present in the dictionary."));
    // FT1 coverage gap: every BcException marker's decoded line is present and a positive
    // integer (wire_line_to_display adds 1, so a valid decode is always >= 1).
    for d in m.data.iter().filter(|d| d["type"] == "BcException") {
        let line = d["line"].as_i64().expect("BcException data.line must be an integer");
        assert!(line >= 1, "BcException line must be >= 1 (decoded), got {line}");
    }
    // Object names must be quoted EXACTLY ONCE (no double-quoting): no BcCall/BcException
    // `data.object` may contain a doubled `""`, and at least one must carry a single quoted
    // segment (with-exception has space-named objects like "System Action Triggers").
    let objects: Vec<&str> = m.data.iter().filter_map(|d| d["object"].as_str()).collect();
    assert!(!objects.iter().any(|o| o.contains("\"\"")), "no marker object may contain a doubled quote (double-quoting bug)");
    assert!(objects.iter().any(|o| o.contains('"')), "expected at least one single-quoted object segment (space-named object)");
}

use bc_mdc_converter::{convert, OutputFormat};

fn assert_valid_firefox(name: &str) {
    let out = convert(&load_zip_bytes(name), OutputFormat::Firefox, &EmitOptions::default()).unwrap();
    let p: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let meta = &p["meta"];
    assert_eq!(meta["preprocessedProfileVersion"], 54, "{name} v54");
    assert_eq!(meta["version"], 34);
    assert_eq!(meta["categories"][0]["color"], "grey", "{name} cat0 grey");
    assert_eq!(meta["categories"][0]["subcategories"][0], "Other");
    assert!(meta["markerSchema"].as_array().unwrap().len() >= 3);
    let th = &p["threads"][0];
    assert_eq!(th["showMarkersInTimeline"], true, "{name} showMarkersInTimeline");
    // These fields are required by the v54->66 upgrader chain (nativeSymbols -> upgrader 56 reads
    // .length); their absence makes profiler.firefox.com reject the profile with
    // "Cannot read properties of undefined (reading 'length')".
    assert!(th["nativeSymbols"].is_object(), "{name} nativeSymbols present");
    assert_eq!(th["nativeSymbols"]["length"], 0, "{name} nativeSymbols empty");
    assert!(th["pausedRanges"].is_array(), "{name} pausedRanges present array");
    assert!(th["registerTime"].is_number(), "{name} registerTime present");
    let s = &th["samples"];
    assert_eq!(s["weightType"], "tracing-ms", "{name} weightType");
    let stacks = s["stack"].as_array().unwrap();
    let weights = s["weight"].as_array().unwrap();
    let times = s["time"].as_array().unwrap();
    assert_eq!(stacks.len(), weights.len());
    assert_eq!(stacks.len(), times.len());
    assert!(stacks.len() >= 1, "{name} >=1 sample");
    // time non-decreasing
    let tv: Vec<f64> = times.iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(tv.windows(2).all(|w| w[0] <= w[1]), "{name} time non-decreasing");
    // every stack index resolves or is null; every prefix is lower or null
    let n_stack = th["stackTable"]["frame"].as_array().unwrap().len();
    for st in stacks { if !st.is_null() { assert!((st.as_u64().unwrap() as usize) < n_stack); } }
    let prefixes = th["stackTable"]["prefix"].as_array().unwrap();
    for (i, px) in prefixes.iter().enumerate() {
        if !px.is_null() { assert!((px.as_u64().unwrap() as usize) < i, "{name} prefix topo"); }
    }
    // no BcSql markers, no counters (honest SQL stance)
    let mdata = th["markers"]["data"].as_array().unwrap();
    assert!(!mdata.iter().any(|d| d["type"] == "BcSql"), "{name} zero BcSql");
    assert!(p.get("counters").is_none(), "{name} no counters");
    // every marker data.type is one of the registered schemas
    let schema_names: Vec<&str> = meta["markerSchema"].as_array().unwrap().iter()
        .map(|s| s["name"].as_str().unwrap()).collect();
    for d in mdata { if !d.is_null() {
        assert!(schema_names.contains(&d["type"].as_str().unwrap()), "{name} marker type registered"); } }
}

#[test] fn firefox_valid_instr()          { assert_valid_firefox("instr"); }
#[test] fn firefox_valid_tiny()           { assert_valid_firefox("tiny"); }
#[test] fn firefox_valid_mixed_objects()  { assert_valid_firefox("mixed-objects"); }
#[test] fn firefox_valid_with_exception() { assert_valid_firefox("with-exception"); }
#[test] fn firefox_valid_long_report()    { assert_valid_firefox("long-report"); }

#[test]
fn firefox_output_is_deterministic() {
    let a = convert(&load_zip_bytes("tiny"), OutputFormat::Firefox, &EmitOptions::default()).unwrap();
    let b = convert(&load_zip_bytes("tiny"), OutputFormat::Firefox, &EmitOptions::default()).unwrap();
    assert_eq!(a, b, "same input -> byte-identical firefox output");
}

use bc_mdc_converter::emit::firefox::sweep::temporal_nesting;

/// ff-m7 GATE 2: the whole point of the temporal-sweep fix. Every emitted `BcCall` marker's
/// `selfMs` must be non-negative for ALL 5 fixtures. The old `span − Σ aggregation-child durations`
/// computation went negative (instr 418, tiny 35, mixed-objects 83, with-exception 44,
/// long-report 47) because `child_ixs` accumulates children across MULTIPLE executions of a looped
/// call; the sweep's per-interval self-time is exact and non-negative by construction (Firefox emitter spec §7.1).
#[test]
fn no_negative_self_ms_in_any_fixture() {
    for name in ["instr", "tiny", "mixed-objects", "with-exception", "long-report"] {
        let out = convert(&load_zip_bytes(name), OutputFormat::Firefox, &EmitOptions::default()).unwrap();
        let p: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let mdata = p["threads"][0]["markers"]["data"].as_array().unwrap();
        for d in mdata {
            if d["type"] == "BcCall" {
                let self_ms = d["selfMs"].as_f64().expect("BcCall selfMs must be a number");
                assert!(self_ms >= 0.0,
                    "{name}: negative selfMs {self_ms} for {} (temporal-sweep self-time must be >= 0)",
                    d["object"]);
            }
        }
    }
}

/// ff-m7 GATE 3 (temporal containment) + GATE 5 (topological). Synthetic case where the
/// V8-AGGREGATION `parent_ix` DIFFERS from the true temporal parent, proving the stackTable prefix
/// follows the temporal SWEEP, not `parent_ix`: inv 2 is temporally nested in inv 1
/// ([2.0,3.0]ms ⊂ [1.0,9.0]ms) but its aggregation `parent_ix` is deliberately set to the root (0).
/// The prefix must be node 1 (temporal), not node 0 (aggregation). Also asserts full [start,end]
/// containment and strictly-lower (topological) prefix indices.
#[test]
fn stacktable_prefix_is_temporal_parent() {
    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 100_000,
        invocations: vec![
            inv(0, 80, 0,      100_000, None,    vec![1, 2]),  // root      [0,10]ms
            inv(1, 81, 10_000, 90_000,  Some(0), vec![2]),     // temporal parent of 2  [1,9]ms
            // aggregation parent_ix deliberately = 0 (the ROOT, NOT the true temporal parent 1):
            inv(2, 82, 20_000, 30_000,  Some(0), vec![]),      // temporally nested in 1 [2,3]ms
        ],
        apps: vec![],
        capture: Default::default(),
    };

    // The sweep must find inv 2's temporal parent to be 1 (open.last() at its ENTER), not 0.
    let (tp, _self_ticks) = temporal_nesting(&profile);
    assert_eq!(tp, vec![None, Some(0), Some(1)],
        "temporal parent follows the spans (2 under 1), NOT the aggregation parent_ix (2 under 0)");

    let t = build_tables(&profile, &EmitOptions::default());
    // All three invocations are framed and in ascending index order -> node index == invocation index.
    assert_eq!(t.stacks.prefix, vec![None, Some(0), Some(1)],
        "stackTable prefix must be the temporal parent: node 2 under node 1, NOT the aggregation root 0");

    // GATE 3 + GATE 5 on the synthetic: full [start,end] containment and topological prefixes.
    let mut inv_of_node: Vec<Option<u32>> = vec![None; t.stacks.length];
    for (ix, node) in t.stack_node_of_invocation.iter().enumerate() {
        if let Some(n) = node { inv_of_node[*n as usize] = Some(ix as u32); }
    }
    let mut checked = 0usize;
    for node in 0..t.stacks.length {
        let Some(prefix) = t.stacks.prefix[node] else { continue };
        assert!((prefix as usize) < node, "prefix {prefix} !< node {node} (topological order)");
        let child = &profile.invocations[inv_of_node[node].unwrap() as usize];
        let parent = &profile.invocations[inv_of_node[prefix as usize].unwrap() as usize];
        let cs = child.span.unwrap();
        let ps = parent.span.unwrap();
        assert!(ps.start_ticks <= cs.start_ticks && cs.end_ticks <= ps.end_ticks,
            "containment failed: parent [{},{}] does not contain child [{},{}]",
            ps.start_ticks, ps.end_ticks, cs.start_ticks, cs.end_ticks);
        checked += 1;
    }
    assert_eq!(checked, 2, "two nested stack nodes expected");
}

/// ff-m7 GATE 3 on REAL data (`tiny`). The rigorous, always-true property the sweep guarantees for
/// a temporal parent is that the parent is OPEN at the child's ENTER: `parent.start <= child.start`
/// (parent entered no later) and `child.start < parent.end` (parent has not yet exited — the
/// EXIT-before-ENTER tie-break makes this strict). The full `child.end <= parent.end` end-containment
/// can be violated by sub-microsecond instrumentation noise in raw spans (a child whose recorded end
/// overshoots its parent's by a few 100ns ticks), so it is asserted on the synthetic above, not here.
#[test]
fn stacktable_prefix_parent_is_open_at_child_start_on_fixture() {
    let snap = decode_snapshot(&load_zip_bytes("tiny")).unwrap();
    let profile = build_profile(&snap.contexts);
    let t = build_tables(&profile, &EmitOptions::default());

    let mut inv_of_node: Vec<Option<u32>> = vec![None; t.stacks.length];
    for (ix, node) in t.stack_node_of_invocation.iter().enumerate() {
        if let Some(n) = node { inv_of_node[*n as usize] = Some(ix as u32); }
    }

    let mut checked = 0usize;
    for node in 0..t.stacks.length {
        let Some(prefix) = t.stacks.prefix[node] else { continue };
        assert!((prefix as usize) < node, "prefix {prefix} !< node {node} (topological order)");
        let child = &profile.invocations[inv_of_node[node].unwrap() as usize];
        let parent = &profile.invocations[inv_of_node[prefix as usize].unwrap() as usize];
        let cs = child.span.expect("framed child has a span");
        let ps = parent.span.expect("framed parent has a span");
        assert!(ps.start_ticks <= cs.start_ticks,
            "parent.start {} > child.start {} (node {node})", ps.start_ticks, cs.start_ticks);
        assert!(cs.start_ticks < ps.end_ticks,
            "temporal parent must be OPEN at child start: child.start {} !< parent.end {} (node {node})",
            cs.start_ticks, ps.end_ticks);
        checked += 1;
    }
    assert!(checked >= 1, "expected at least one non-root (nested) stack node in tiny");
}

/// ff-m7 GATE 4 (self-time conservation, Firefox emitter spec §7.1). Σ self_ticks == Σ sample weights over the
/// leaf-covered (non-idle) sub-intervals — exact by construction: every non-idle sample weight is
/// one sub-interval whose duration was added to exactly one invocation's self_ticks, so no self-time
/// is lost or double-counted. `tiny` is a sparse multi-root capture (~810ms of genuine idle gaps
/// between activity bursts, ~50ms of framed leaf time), so total covered time (`end - t0`) is NOT
/// all leaf-covered; the no-idle synthetic below asserts the full `Σself == Σweight == covered`.
#[test]
fn self_time_conservation_on_fixture() {
    let snap = decode_snapshot(&load_zip_bytes("tiny")).unwrap();
    let profile = build_profile(&snap.contexts);
    let t = build_tables(&profile, &EmitOptions::default());
    let t0 = profile_t0(&profile);
    let (_tp, self_ticks) = temporal_nesting(&profile);
    let s = build_samples(&profile, &t.stack_node_of_invocation, t0);

    let self_sum_ms = self_ticks.iter().sum::<i64>() as f64 / 10000.0;
    let weight_nonidle: f64 = s.stack.iter().zip(&s.weight)
        .filter_map(|(st, w)| st.map(|_| *w)).sum();
    assert!((self_sum_ms - weight_nonidle).abs() < 1e-6,
        "Σself_ticks {self_sum_ms}ms != Σnon-idle sample weight {weight_nonidle}ms");
    assert!(self_ticks.iter().all(|&x| x >= 0), "no self_ticks may be negative");
}

/// ff-m7 GATE 4 full form on a no-idle synthetic (Firefox emitter spec §8 worked example): with the root open
/// across the entire capture there are no idle gaps, so Σ self_ticks == Σ ALL sample weights ==
/// total covered time (`end - t0`) == the root's running time.
#[test]
fn self_time_conservation_full_no_idle_synthetic() {
    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 1_300_000,
        invocations: vec![
            inv(0, 80,   0,         1_300_000, None,    vec![1, 3]),
            inv(1, 80,   200_000,   900_000,   Some(0), vec![2]),
            inv(2, 37,   300_000,   600_000,   Some(1), vec![]),
            inv(3, 6620, 1_000_000, 1_250_000, Some(0), vec![]),
        ],
        apps: vec![],
        capture: Default::default(),
    };
    let t0 = profile_t0(&profile);
    let (_tp, self_ticks) = temporal_nesting(&profile);
    let stack_of = vec![Some(0u32), Some(1), Some(2), Some(3)];
    let s = build_samples(&profile, &stack_of, t0);

    let self_sum_ms = self_ticks.iter().sum::<i64>() as f64 / 10000.0;
    let weight_all: f64 = s.weight.iter().sum();
    let covered = tick_to_ms(profile.end_time_ticks, t0);
    assert!((self_sum_ms - weight_all).abs() < 1e-9, "Σself {self_sum_ms}ms != Σweight {weight_all}ms");
    assert!((self_sum_ms - covered).abs() < 1e-9, "Σself {self_sum_ms}ms != covered {covered}ms (130ms)");
    // exact per-invocation self-times (Firefox emitter spec §8): root=35, inv1=40, inv2=30, inv3=25 (ms)
    assert_eq!(self_ticks, vec![350_000, 400_000, 300_000, 250_000]);
}

use bc_mdc_converter::emit::firefox::sweep::run_sweep;

#[test]
fn sweep_effective_end_tracks_clamp_and_membership() {
    let mut no_span = inv(2, 99, 0, 0, None, vec![]);
    no_span.span = None; // never enters the sweep
    let profile = Profile {
        start_time_ticks: 0, end_time_ticks: 1_000_000,
        invocations: vec![
            inv(0, 80, 0, 1_000_000, None, vec![1]),            // raw end used
            inv_ic(1, 80, 0, 3_000_000, Some(0), vec![], true), // incomplete: clamped to parent end
            no_span,
        ],
        apps: vec![],
        capture: Default::default(),
    };
    let s = run_sweep(&profile);
    assert_eq!(s.effective_end[0], Some(1_000_000)); // in sweep, raw end
    assert_eq!(s.effective_end[1], Some(1_000_000)); // clamp changed 3_000_000 -> 1_000_000
    assert_eq!(s.effective_end[2], None);            // no span -> not in sweep
    assert_eq!(s.effective_end.len(), 3);
}
