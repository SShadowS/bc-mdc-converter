// algorithm spec §6 Phase 4 — AggregateFrames (the hard part). A faithful serial port of the
// reference aggregation pass. Collapse every group of frames representing the same call
// made repeatedly (same parent + same called line — think loop bodies) into one representative
// frame — the one with the LOWEST `ExecutionIndex` in the group — accumulating hit counts and
// (conditionally) execution times, then repair the parent/child links that collapsing broke.
//
// Representation (see `V8State`): frames live in `frames: Vec<CallFrameDef>` keyed by
// `by_index: ExecutionIndex -> slot`. Aggregation NEVER physically removes a frame; it marks
// merged-away indexes in a `removed`/`merged` set. `by_index` therefore stays stable and every
// index — including merged ones — remains resolvable (required: §6.7 relink and the emit hash
// chain both traverse original parent pointers that can point at merged-away frames).
//
// Borrow discipline: the C# graph is shared-mutable; Rust is not. Every mutation copies the
// scalars/lists it needs out of `state` first (releasing the borrow), then re-borrows `&mut`
// to write back. We never hold two `&mut self.frames[..]` at once.

use std::collections::HashSet;

use crate::emit::v8::V8State;
use crate::ir::line_opt_eq;

/// §6.2 top-level driver. Three passes over ascending `ExecutionIndex`: (a) merge, (b) removal
/// and scrub, (c) orphan relink. `merged` is the running "these got folded away" set (the C#'s
/// `aggregated`).
pub(crate) fn aggregate_frames(state: &mut V8State) {
    let mut merged: HashSet<u32> = HashSet::new();

    // (a) merge pass — iterate frames in ascending ExecutionIndex order.
    let order: Vec<u32> = state.frames_execution_indexes_ascending();
    for ix in &order {
        if !merged.contains(ix) {
            aggregate_into_frame(state, *ix, &mut merged); // §6.3 worklist
        }
    }

    // (b) removal pass — logically remove each merged frame, then scrub its index from every
    //     surviving child list (§6.6).
    for ix in &merged {
        state.remove_from_survivors(*ix);
        remove_frame_from_all_callers(state, *ix); // §6.6
    }

    // (c) orphan-relink pass — over survivors (§6.7).
    let survivors: Vec<u32> = state.survivor_indexes();
    for ix in survivors {
        verify_for_missing_caller_link(state, ix, &merged); // §6.7
    }
}

/// §6.3 AggregateIntoFrame (worklist form). `to_ix` is the representative being built up.
/// Five steps, strictly ordered.
fn aggregate_into_frame(state: &mut V8State, to_ix: u32, merged: &mut HashSet<u32>) {
    if merged.contains(&to_ix) {
        return;
    }
    // Children of merged siblings — candidates for adoption in step 4.
    let mut orphan_children: HashSet<u32> = HashSet::new();

    // 1) fold in every later same-group sibling of `to` (§6.4, ascending index).
    let sibs = get_aggregation_context_for_frame(state, to_ix, merged);
    for sib in sibs {
        if merged.insert(sib) {
            // newly marked merged: collect its children, then sum hits/time into `to` (§6.5).
            let sib_children = state.frame(sib).called_frame_indexes.clone();
            for c in sib_children {
                orphan_children.insert(c);
            }
            merge_one(state, sib, to_ix);
        }
    }

    // 2) a frame must not list itself as its own child.
    {
        let f = state.frame_mut(to_ix);
        if let Some(pos) = f.called_frame_indexes.iter().position(|&c| c == to_ix) {
            f.called_frame_indexes.remove(pos);
            if !f.called_frame_indexes.is_empty() {
                f.called_frame_indexes.sort_unstable();
            }
        }
    }

    // 3) recurse into `to`'s (own) children (§6.3b), over a snapshot taken now — step-4
    //    adoptions happen AFTER this and are intentionally not visited here.
    let children = state.frame(to_ix).called_frame_indexes.clone();
    aggregate_called_frames(state, &children, merged);

    // 4) adopt orphaned children of the folded siblings whose ORIGINAL parent line matches
    //    `to`'s line (§6.3 step 4). Guard a `None` parent (no adoption).
    if !orphan_children.is_empty() {
        let to_line = state.frame(to_ix).line_definition;
        let before = state.frame(to_ix).called_frame_indexes.len();
        for &oc in &orphan_children {
            if merged.contains(&oc) {
                continue;
            }
            if state.frame(to_ix).called_frame_indexes.contains(&oc) {
                continue;
            }
            let Some(oc_frame) = state.try_frame(oc) else { continue };
            let Some(oc_parent_ix) = oc_frame.parent else { continue }; // None parent → no adoption
            let oc_parent_line = state.try_frame(oc_parent_ix).and_then(|pf| pf.line_definition);
            if line_opt_eq(oc_parent_line, to_line) {
                state.frame_mut(to_ix).called_frame_indexes.push(oc);
            }
        }
        if state.frame(to_ix).called_frame_indexes.len() > before {
            state.frame_mut(to_ix).called_frame_indexes.sort_unstable();
        }
    }

    // 5) ensure frame time >= sum of child times (§6.8).
    adjust_frame_execution_time(state, to_ix);
}

/// §6.3b AggregateCalledFrames — the mutual recursion: aggregate each not-yet-merged child.
/// `called` is a snapshot of the parent's child list taken by the caller.
fn aggregate_called_frames(state: &mut V8State, called: &[u32], merged: &mut HashSet<u32>) {
    for &num in called {
        if !merged.contains(&num) && state.by_index.contains_key(&num) {
            aggregate_into_frame(state, num, merged);
        }
    }
}

/// §6.4 GetAggregationContextForFrame — `to`'s later same-group siblings, ascending. A sibling
/// is any live frame with strictly greater `ExecutionIndex`, the same parent (frame-equality;
/// both-None equal), and the same `LineDefinition` (loose `(from.line, id)` equality). The
/// strictly-greater bound + ascending driver order is what makes the lowest-index frame the
/// representative.
fn get_aggregation_context_for_frame(
    state: &V8State,
    to_ix: u32,
    merged: &HashSet<u32>,
) -> Vec<u32> {
    let to = state.frame(to_ix);
    let to_parent = to.parent;
    let to_line = to.line_definition;

    let mut result: Vec<u32> = Vec::new();
    for f in &state.frames {
        let ix = f.execution_index;
        if ix <= to_ix {
            continue;
        }
        if merged.contains(&ix) {
            continue;
        }
        // Parent frame-equality: parents are stored as indexes and `ExecutionIndex` is a unique
        // identity, so `Option<u32>` equality IS frame-equality of the parent frames.
        if f.parent != to_parent {
            continue;
        }
        if !line_opt_eq(f.line_definition, to_line) {
            continue;
        }
        result.push(ix);
    }
    result.sort_unstable();
    result
}

/// §6.5 AggregateIntoFrame (merge form) + AggregateHitCountAndExecutionTime. Folds one sibling
/// `from` into representative `to`. The two subtle flags live here (truth table §6.5):
/// (1) `from` IsIncompleteMeasurement → add hits YES, add time YES;
/// (2) else if `from` is self-referential (lists its own index) → add hits YES, add time NO;
/// (3) else → add hits YES, add time YES.
/// `to` is never added to `aggregated`; only `from` (a sibling) is folded in.
fn merge_one(state: &mut V8State, from_ix: u32, to_ix: u32) {
    if from_ix == to_ix {
        return;
    }
    let incomplete = state.contexts_incomplete[from_ix as usize];
    let (add_hits, add_time) = if incomplete {
        (true, true)
    } else if state.frame(from_ix).called_frame_indexes.contains(&from_ix) {
        (true, false) // self-referential: hits yes, time NO (avoid double-counting)
    } else {
        (true, true)
    };

    let (fh, ft) = {
        let f = state.frame(from_ix);
        (f.hit_count, f.aggregated_execution_time)
    };
    let to = state.frame_mut(to_ix);
    if add_hits {
        to.hit_count += fh;
    }
    if add_time {
        to.aggregated_execution_time += ft;
    }
}

/// §6.6 RemoveFrameFromAllCallers — scrub merged-away index `x` from every child list. (A
/// surviving child is never a scrub target, so scrubbing all frames vs only survivors is
/// output-equivalent; we scrub all.)
fn remove_frame_from_all_callers(state: &mut V8State, x: u32) {
    for frame in &mut state.frames {
        frame.called_frame_indexes.retain(|&c| c != x);
    }
}

/// §6.7 VerifyForMissingCallerLink — for a surviving frame `f`, make sure its parent actually
/// lists it; if the parent was merged away, re-attach `f` to the representative that replaced
/// it and propagate `f`'s time up the ancestry (stopping before the root, `ExecutionIndex 0`).
fn verify_for_missing_caller_link(state: &mut V8State, f_ix: u32, merged: &HashSet<u32>) {
    let Some(p_ix) = state.frame(f_ix).parent else { return };
    // `parent = dict.get(p)` — may be a merged (but still resolvable) frame.
    let Some(parent) = state.try_frame(p_ix) else { return };
    if parent.called_frame_indexes.contains(&f_ix) {
        return; // link already intact
    }

    let target_ix = if merged.contains(&p_ix) {
        // parent was merged away — find the representative sibling that replaced it.
        match find_caller_frame_for_child_frame(state, p_ix, merged) {
            Some(t) => t,
            None => return,
        }
    } else {
        p_ix
    };

    // Re-attach `f` to the target, keeping the child list sorted ascending.
    {
        let t = state.frame_mut(target_ix);
        t.called_frame_indexes.push(f_ix);
        t.called_frame_indexes.sort_unstable();
    }

    // Propagate `f`'s time up the ancestry: each ancestor's AggregatedExecutionTime and its
    // caller-statement tick gain `f`'s time. Stop BEFORE the root (ExecutionIndex 0 is left
    // as-is).
    let f_time = state.frame(f_ix).aggregated_execution_time;
    let mut anc = Some(target_ix);
    while let Some(a_ix) = anc {
        if a_ix == 0 {
            break;
        }
        let a_parent = {
            let a = state.frame_mut(a_ix);
            a.aggregated_execution_time += f_time;
            a.parent
        };
        if let Some(tick) = state.caller_position_tick_mut(a_ix) {
            tick.execution_time += f_time;
        }
        anc = a_parent;
    }
}

/// §6.7 FindCallerFrameForChildFrame — `child_ix` is `f`'s ORIGINAL (merged-away) parent. Look
/// at that frame's parent (`f`'s grandparent) and, among the grandparent's children, return the
/// one whose `LineDefinition` matches the merged parent's line — the representative the merged
/// parent folded into. `None` if the grandparent is missing or itself merged.
fn find_caller_frame_for_child_frame(
    state: &V8State,
    child_ix: u32,
    merged: &HashSet<u32>,
) -> Option<u32> {
    let child = state.try_frame(child_ix)?;
    let gp_ix = child.parent?;
    if merged.contains(&gp_ix) {
        return None; // grandparent also merged — give up
    }
    let child_line = child.line_definition;
    let gp_children = state.try_frame(gp_ix)?.called_frame_indexes.clone();
    for ci in gp_children {
        if let Some(cand) = state.try_frame(ci) {
            if line_opt_eq(cand.line_definition, child_line) {
                return Some(ci);
            }
        }
    }
    None
}

/// §6.8 AdjustFrameExecutionTime — enforce frame time >= Σ children time. If short, bump the
/// frame to the child sum and add the same diff to the frame's caller-statement tick (keeping
/// the two independent accumulators consistent). Runs at the end of every worklist invocation,
/// so it fires bottom-up as the recursion unwinds.
fn adjust_frame_execution_time(state: &mut V8State, frame_ix: u32) {
    let children = state.frame(frame_ix).called_frame_indexes.clone();
    let mut child_sum: i64 = 0;
    for c in &children {
        if let Some(cf) = state.try_frame(*c) {
            child_sum += cf.aggregated_execution_time;
        }
    }
    let cur = state.frame(frame_ix).aggregated_execution_time;
    if cur < child_sum {
        let diff = child_sum - cur;
        state.frame_mut(frame_ix).aggregated_execution_time += diff;
        if let Some(tick) = state.caller_position_tick_mut(frame_ix) {
            tick.execution_time += diff;
        }
    }
}
