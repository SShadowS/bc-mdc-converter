mod common;
use bc_mdc_converter::build_ir::build_profile;
use bc_mdc_converter::emit::v8::{self, frame_identifier_for};
use bc_mdc_converter::hash::{CombineForm, LOCKED_FORM};
use bc_mdc_converter::mdc::decode::decode_snapshot;
use common::{load_reference, load_zip_bytes};

#[test]
fn some_combine_form_matches_all_706_frame_identifiers() {
    let reference = load_reference("instr");
    let mut want: std::collections::HashMap<i64, i64> = Default::default();
    for n in reference["nodes"].as_array().unwrap() {
        want.insert(n["id"].as_i64().unwrap(), n["frameIdentifier"].as_i64().unwrap());
    }
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let mut state = v8::build_state(&profile);
    v8::phase3_attribute_and_prune(&mut state);
    v8::aggregate_frames(&mut state);

    let forms = [
        CombineForm::RotateXor,
        CombineForm::Multiply,
        CombineForm::RotateXorSwapped,
        CombineForm::MultiplySwapped,
    ];
    let mut winner = None;
    let mut best: (Option<CombineForm>, usize) = (None, 0);
    for &form in &forms {
        let survivors = state.survivor_indexes();
        let matched = survivors
            .iter()
            .filter(|&&ix| {
                let got = frame_identifier_for(&state, ix, form) as i64;
                // frameIdentifier is i32; reference stores it as (possibly negative) JSON int
                let want_v = *want.get(&(ix as i64 + 1)).unwrap();
                (got as i32) as i64 == want_v
            })
            .count();
        if matched > best.1 {
            best = (Some(form), matched);
        }
        if matched == survivors.len() {
            winner = Some(form);
            break;
        }
    }
    assert!(
        winner.is_some(),
        "no Hash.Combine candidate matched all 706 — widen the family (open question #1); best was {:?} matching {}/706",
        best.0,
        best.1
    );
    println!("frameIdentifier winner: {:?}", winner.unwrap());
}

#[test]
fn locked_form_matches_node1_reference_frame_identifier() {
    let snap = decode_snapshot(&load_zip_bytes("instr")).unwrap();
    let profile = build_profile(&snap.contexts);
    let mut state = v8::build_state(&profile);
    v8::phase3_attribute_and_prune(&mut state);
    v8::aggregate_frames(&mut state);

    // node id = execution_index + 1; node 1 => execution_index 0.
    let got = frame_identifier_for(&state, 0, LOCKED_FORM);
    assert_eq!(got, 602714591, "node 1 frameIdentifier under LOCKED_FORM");
}
