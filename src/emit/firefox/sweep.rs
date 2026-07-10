// Boundary-sweep sample stream (Firefox emitter spec §1.3) — the CORE mechanism
// (§0: "everything else hangs off it"). Every eligible invocation span becomes two boundary
// events (ENTER at start_ticks, EXIT at end_ticks); sweeping the sorted events with an explicit
// "open" stack yields the exact active leaf invocation in every sub-interval, so a function's
// flame-graph self time falls out exactly (§7.1) — zero sampling-interval quantization error,
// unlike Firefox's own re-sampling importers (§1.4).

use crate::emit::firefox::model::SamplesTable;
use crate::emit::firefox::tick_to_ms;
use crate::ir::Profile;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Exit,
    Enter,
}

struct Event {
    tick: i64,
    kind: Kind,
    ix: u32, // invocation execution index (== .mdc ordinal)
}

/// One closed boundary-sweep sub-interval `[start, end)` and the invocation that is the active
/// leaf across it (`None` for an idle gap where no invocation is open). Kept in raw ticks and at
/// invocation granularity so the same nesting pass feeds samples, temporal parents, and
/// self-times — the stackTable mapping (`stack_node_of_invocation`) is applied later, in
/// `SweepResult::build_samples`, because the stack nodes don't exist yet when the sweep runs.
struct SubInterval {
    start: i64,
    end: i64,
    leaf: Option<u32>, // active leaf invocation index, or None for an idle gap
}

/// Everything the ONE boundary sweep produces (Firefox emitter spec §1.3/§7.1) — computed in a single pass over
/// the SAME clamped event stream so all three outputs are mutually consistent:
/// - `temporal_parent[ix]` — invocation `ix`'s TRUE temporal parent (`open.last()` captured at its
///   ENTER, before `ix` is pushed), or `None` for a temporal root / an invocation never entered
///   (unframed or dropped by the incomplete-clamp). This is the actual call-tree edge, NOT the IR's
///   V8-aggregation `parent_ix`. Because `open` only ever holds framed invocations, it is already
///   the nearest FRAMED temporal ancestor (unframed pass-throughs are skipped for free).
/// - `self_ticks[ix]` — exact flame-graph self time of `ix` in ticks: the sum of every sub-interval
///   where `ix` is the leaf. Non-negative and exact by construction (Firefox emitter spec §7.1), unlike the old
///   `span − Σ aggregation-child durations` which over-counted looped children and went negative.
/// - `sub_intervals` — the per-interval leaves, deferred until `stack_node_of_invocation` exists.
/// - `effective_end[ix]` — `Some(post-clamp end tick)` iff the invocation produced ENTER/EXIT events (the
///   Firefox emitter spec §1.3 eligibility rules: has a span, has a valid object, positive width
///   after the incomplete-clamp); `None` otherwise. RAW ticks, not rebased. ir-json's
///   `inSweep` is `is_some()`; its `clampedEndTicks` is this value when it differs
///   from the raw span end (spec §3.4).
pub struct SweepResult {
    pub temporal_parent: Vec<Option<u32>>,
    pub self_ticks: Vec<i64>,
    sub_intervals: Vec<SubInterval>,
    pub effective_end: Vec<Option<i64>>,
}

/// Runs the boundary sweep ONCE (Firefox emitter spec §1.3) and returns its full result. Needs only the
/// invocation spans — NOT the stackTable — so it can be called BEFORE `build::build_tables`
/// (which needs `temporal_parent` to set stackTable prefixes). `build_samples` is applied afterwards
/// via `SweepResult::build_samples`, once the stack nodes exist.
pub fn run_sweep(profile: &Profile) -> SweepResult {
    let (mut events, effective_end) = boundary_events(profile);
    events.sort_by(sort_key);

    let n = profile.invocations.len();
    let mut temporal_parent: Vec<Option<u32>> = vec![None; n];
    let mut self_ticks: Vec<i64> = vec![0; n];
    let mut sub_intervals: Vec<SubInterval> = Vec::new();

    // `open` is the stack of currently-open invocations, deepest (current active leaf) last.
    let mut open: Vec<u32> = Vec::new();
    let mut prev_tick: Option<i64> = None;

    for ev in &events {
        if let Some(pt) = prev_tick {
            if ev.tick > pt {
                // One closed sub-interval [prev_tick, ev.tick): attribute its duration to the
                // active leaf's self time, and record it for the deferred sample mapping.
                let leaf = open.last().copied();
                if let Some(leaf_ix) = leaf {
                    self_ticks[leaf_ix as usize] += ev.tick - pt;
                }
                sub_intervals.push(SubInterval { start: pt, end: ev.tick, leaf });
            }
        }
        match ev.kind {
            Kind::Enter => {
                // Capture the temporal parent BEFORE pushing `ix`: `open.last()` is the innermost
                // call still executing when `ix` starts, i.e. `ix`'s true caller (Firefox emitter spec §7.1).
                temporal_parent[ev.ix as usize] = open.last().copied();
                open.push(ev.ix);
            }
            Kind::Exit => {
                // It is the current top by construction (the tie-break guarantees this) — but
                // remove by value to be safe (Firefox emitter spec §1.3).
                if let Some(pos) = open.iter().rposition(|&x| x == ev.ix) {
                    open.remove(pos);
                }
            }
        }
        prev_tick = Some(ev.tick);
    }

    SweepResult { temporal_parent, self_ticks, sub_intervals, effective_end }
}

impl SweepResult {
    /// Maps the sweep's per-interval leaf invocations through `stack_node_of_invocation[ix]` (the
    /// stackTable row for invocation `ix`, built by `build::build_tables`, FT2) to produce the
    /// sample stream. `t0` is `profile_t0(profile)` (FT0). `stack == None` for a sub-interval where
    /// no invocation is open (idle gap between disjoint top-level roots) — never synthesized as a
    /// fake frame.
    pub fn build_samples(
        &self,
        stack_node_of_invocation: &[Option<u32>],
        t0: i64,
    ) -> SamplesTable {
        let mut stack: Vec<Option<u32>> = Vec::with_capacity(self.sub_intervals.len());
        let mut time: Vec<f64> = Vec::with_capacity(self.sub_intervals.len());
        let mut weight: Vec<f64> = Vec::with_capacity(self.sub_intervals.len());

        for sub in &self.sub_intervals {
            let leaf = sub.leaf.and_then(|ix| stack_node_of_invocation[ix as usize]);
            stack.push(leaf);
            time.push(tick_to_ms(sub.start, t0));
            weight.push(tick_to_ms(sub.end, t0) - tick_to_ms(sub.start, t0));
        }

        let length = stack.len();
        SamplesTable { stack, time, weight, weight_type: "tracing-ms".to_string(), length }
    }
}

/// Thin wrapper preserved for call sites that only want the sample stream (e.g. `tests/firefox.rs`
/// and any single-shot caller): runs the sweep and maps its leaves in one call. `emit` does NOT use
/// this — it calls `run_sweep` once and reuses the result for samples, prefixes, AND self-times.
pub fn build_samples(
    profile: &Profile,
    stack_node_of_invocation: &[Option<u32>],
    t0: i64,
) -> SamplesTable {
    run_sweep(profile).build_samples(stack_node_of_invocation, t0)
}

/// Standalone accessor for the sweep's temporal nesting: `(temporal_parent, self_ticks)`, WITHOUT
/// needing the stackTable. `build::build_tables` calls this to set stackTable prefixes from the
/// TRUE temporal parent (not the IR's V8-aggregation `parent_ix`); `emit` gets the same data from
/// `run_sweep` directly (one sweep) for self-times.
pub fn temporal_nesting(profile: &Profile) -> (Vec<Option<u32>>, Vec<i64>) {
    let r = run_sweep(profile);
    (r.temporal_parent, r.self_ticks)
}

/// Two boundary events per eligible invocation (Firefox emitter spec §1.3). An invocation is eligible when
/// it has a real span, a valid object (object-less invocations create no stackTable node and are
/// never `open`, so a child's `temporal_parent` skips past them to the nearest framed ancestor),
/// and a strictly positive duration after the incomplete-measurement clamp.
fn boundary_events(profile: &Profile) -> (Vec<Event>, Vec<Option<i64>>) {
    let mut events = Vec::with_capacity(profile.invocations.len() * 2);
    let mut effective_end: Vec<Option<i64>> = vec![None; profile.invocations.len()];
    for inv in &profile.invocations {
        if inv.object.is_none() {
            continue;
        }
        let Some(span) = inv.span else { continue };
        let start = span.start_ticks;
        let mut end = span.end_ticks;

        // Incomplete clamp (Firefox emitter spec §1.3): an is_incomplete invocation's end_ticks may be
        // unreliable/over-long/missing. Precedence: clamp to the parent's end_ticks when the
        // parent has a usable span; else (a ROOT, or a parent with no span) set the end to the
        // max end_ticks over the transitive descendants (the deepest real coverage we can
        // trust) — the textbook "capture cut off mid-outermost-call" case. If there are no
        // framed descendants either, leave the end as-is. The drop-if-`end <= start` guard
        // below still applies; a span that cannot be repaired is dropped from the sweep (it
        // still gets a marker later, FT4). Never let end precede start.
        if inv.is_incomplete {
            let parent_end = inv
                .parent_ix
                .and_then(|pix| profile.invocations[pix as usize].span)
                .map(|s| s.end_ticks);
            if let Some(pe) = parent_end {
                end = end.min(pe);
            } else if let Some(max_desc) = max_descendant_end(profile, inv.index) {
                end = max_desc;
            }
        }

        if end <= start {
            continue; // zero/negative width after clamp -> drop from the sweep
        }

        effective_end[inv.index as usize] = Some(end);
        events.push(Event { tick: start, kind: Kind::Enter, ix: inv.index });
        events.push(Event { tick: end, kind: Kind::Exit, ix: inv.index });
    }
    (events, effective_end)
}

/// Max `end_ticks` over the transitive descendants of invocation `ix` that have a span (Firefox emitter spec
/// §1.3, the root branch of the incomplete clamp). Walks `child_ixs` iteratively; `None` if no
/// framed descendant carries a span (caller then leaves the end unchanged).
fn max_descendant_end(profile: &Profile, ix: u32) -> Option<i64> {
    let mut best: Option<i64> = None;
    let mut stack: Vec<u32> = profile.invocations[ix as usize].child_ixs.clone();
    while let Some(c) = stack.pop() {
        let child = &profile.invocations[c as usize];
        if let Some(sp) = child.span {
            best = Some(best.map_or(sp.end_ticks, |b: i64| b.max(sp.end_ticks)));
        }
        stack.extend(child.child_ixs.iter().copied());
    }
    best
}

/// The tie-break that keeps nesting correct at a shared boundary (Firefox emitter spec §1.3 — the trap): at
/// equal tick, EXITs sort before ENTERs; among EXITs, child before parent (execution index
/// DESC, so the deepest invocation is popped first); among ENTERs, parent before child
/// (execution index ASC, so the parent is opened before its child).
fn sort_key(a: &Event, b: &Event) -> std::cmp::Ordering {
    a.tick
        .cmp(&b.tick)
        .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
        .then_with(|| match a.kind {
            Kind::Exit => b.ix.cmp(&a.ix),
            Kind::Enter => a.ix.cmp(&b.ix),
        })
}

fn kind_rank(k: Kind) -> u8 {
    match k {
        Kind::Exit => 0,
        Kind::Enter => 1,
    }
}
