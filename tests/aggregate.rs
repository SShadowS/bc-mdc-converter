mod common;
use bc_mdc_converter::mdc::decode::decode_snapshot;
use bc_mdc_converter::build_ir::build_profile;
use bc_mdc_converter::emit::v8;
use common::load_zip_bytes;

#[test]
fn phase3_attributes_caller_time_and_prunes() {
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let mut state = v8::build_state(&profile);
    let before = state.tick_count();
    v8::phase3_attribute_and_prune(&mut state);
    let after = state.tick_count();

    // Prune must drop the zero-time statements (every hit line starts at execution_time 0;
    // only those that received attributed callee time survive).
    assert!(after < before, "prune must drop zero-time statements");

    // Prune invariant (algorithm spec §5 step 2): NO surviving tick may still be at execution_time 0.
    assert!(
        state.tick_values().all(|(_, _, exec)| exec > 0),
        "every surviving tick must have execution_time > 0 after prune",
    );

    // Ground-truth anchor: reference node 1's positionTick is exactly
    // {line: 278, column: 7, ticks: 1, executionTime: 7949} — the caller statement (278:7 in
    // CodeUnit_2000000006) for invocation 0's child. Its accumulated time must be 7949.
    assert!(
        state.tick_values().any(|(line, _, exec)| line == 278 && exec == 7949),
        "reference node 1 anchor tick (line 278, execution_time 7949) must be present",
    );

    // Regression pin: Phase 3 in ISOLATION yields 564 surviving ticks — the 550 emitted in
    // the reference profile PLUS 14 whose owner invocation Phase 4 later merges away (their
    // owner is a non-representative of its (parent, called-line) aggregation group, so the
    // tick becomes unreachable at emit). The authoritative 550 is a POST-aggregation count;
    // do NOT "fix" this to 550 — that is checked against the reference in the acceptance /
    // Task 10 emit tests once Phase 4 lands.
    assert_eq!(
        after, 564,
        "Phase-3 survivors (pre-aggregation superset of the 550 emitted; Task 10 checks the authoritative 550 against the reference)",
    );
}

#[test]
fn phase4_yields_706_and_conserves_hits() {
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let mut state = v8::build_state(&profile);
    v8::phase3_attribute_and_prune(&mut state);
    v8::aggregate_frames(&mut state);
    let survivors = state.survivor_indexes();
    assert_eq!(survivors.len(), 706);
    let sum: i32 = survivors.iter().map(|&ix| state.frame(ix).hit_count).sum();
    assert_eq!(sum, 11285); // conservation preserved
}

#[test]
fn phase4_children_match_reference_for_node_1() {
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let mut state = v8::build_state(&profile);
    v8::phase3_attribute_and_prune(&mut state);
    v8::aggregate_frames(&mut state);
    // node id = execution_index + 1; node 1 => execution_index 0; reference children == [2]
    let node1 = state.frame(0);
    let child_ids: Vec<i32> = node1.called_frame_indexes.iter().map(|&c| c as i32 + 1).collect();
    assert_eq!(child_ids, vec![2]);
}

#[test]
fn phase4_children_and_hitcount_match_reference_all_nodes() {
    let reference = common::load_reference("instr");
    let mut want: std::collections::HashMap<i64, (i64, Vec<i64>)> = Default::default();
    for n in reference["nodes"].as_array().unwrap() {
        let id = n["id"].as_i64().unwrap();
        let hc = n["hitCount"].as_i64().unwrap();
        let ch = n["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_i64().unwrap())
            .collect();
        want.insert(id, (hc, ch));
    }
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let mut state = v8::build_state(&profile);
    v8::phase3_attribute_and_prune(&mut state);
    v8::aggregate_frames(&mut state);
    for ix in state.survivor_indexes() {
        let f = state.frame(ix);
        let id = ix as i64 + 1;
        let (whc, wch) = want.get(&id).unwrap_or_else(|| panic!("id {id} missing in reference"));
        assert_eq!(f.hit_count as i64, *whc, "hitCount id {id}");
        let mut got: Vec<i64> = f.called_frame_indexes.iter().map(|&c| c as i64 + 1).collect();
        got.sort_unstable();
        assert_eq!(&got, wch, "children id {id}");
    }
}
