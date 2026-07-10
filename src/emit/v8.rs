// V8 (`.alcpuprofile`) emitter. This file currently holds the working graph rebuilt from
// the IR (algorithm spec §2.2 `CpuProfileCallFrameDefinition`, §2.3
// `PositionTickInfo`) and Phase 3 (§5 UpdateStatementExecutionTimeWithFrameExecutionTime).
// Serialization (Phase 4 aggregation + Phase 5 emit) lands in later tasks.

use std::collections::{HashMap, HashSet};

use crate::emit::v8_json;
use crate::hash::{self, CombineForm};
use crate::ir::{LineDef, LineKey, Profile};
use crate::{EmitOptions, Emitter, Error, Result};

/// algorithm spec §2.2 `CpuProfileCallFrameDefinition` — the working frame (one per surviving call).
/// `hit_indexes` is omitted: it is vestigial upstream (initialized empty, only ever unioned
/// with another equally-empty set, and never seeded with a concrete index — its `Contains`
/// check never fires), so carrying the field would add state that can never do anything.
#[derive(Clone, Debug)]
pub struct CallFrameDef {
    pub line_definition: Option<LineDef>,
    pub parent: Option<u32>,
    pub execution_index: u32,
    pub called_frame_indexes: Vec<u32>,
    pub hit_count: i32,
    pub aggregated_execution_time: i64,
}

/// algorithm spec §2.3 `PositionTickInfo` — per-statement hit + self-time, emitted under a node.
#[derive(Clone, Copy, Debug)]
pub struct PositionTick {
    pub line: i32,
    pub column: i32,
    pub ticks: i32,
    pub execution_time: i64,
}

/// The V8 emitter's working state, rebuilt from the format-neutral IR. Mirrors algorithm spec §2.4
/// (the generator's side state), minus the Phase-2-local `linesWithIndexes` dict (already
/// consumed when the IR's `parent_ix`/`child_ixs` were computed).
pub struct V8State {
    /// Surviving call frames, in ascending `execution_index` order (one per invocation with
    /// a valid object).
    pub(crate) frames: Vec<CallFrameDef>,
    /// keyed by statement line; value = (owning execution_index, tick info).
    pub(crate) lines_with_hits: HashMap<LineKey, (u32, PositionTick)>,
    /// `ExecutionIndex -> (callerLine?, calledLine?)`, for ALL invocations (not just those
    /// with a valid object) so `side[frame.execution_index]` lookups always resolve.
    pub(crate) side: Vec<(Option<LineDef>, Option<LineDef>)>,
    /// Part of the §2.4 working state (mandated by the Task 6 brief's `V8State` shape) but
    /// first consumed by Phase 4 aggregation (Task 7+): `by_index` resolves an
    /// `ExecutionIndex` to its slot in `frames`, and `contexts_incomplete` carries each
    /// context's `IsIncompleteMeasurement` for the §6.5 merge flags. Built here, read there.
    pub(crate) by_index: HashMap<u32, usize>,
    /// `ExecutionIndex -> IsIncompleteMeasurement`, for ALL invocations. See `by_index` above.
    pub(crate) contexts_incomplete: Vec<bool>,
    /// Phase-4 (§6.2b) logical removal set: execution indexes merged away by aggregation.
    /// Frames are NEVER physically removed from `frames` — collapsing shifts no slots, so
    /// `by_index` stays stable and merged frames remain resolvable (mandated for the §6.7
    /// relink and the emit-time hash chain, which traverse original parent pointers that may
    /// point at merged-away frames). `survivor_indexes()` is `frames` minus this set.
    pub(crate) removed: HashSet<u32>,
}

impl V8State {
    /// Number of surviving statement ticks in `lines_with_hits` (the working position-tick
    /// set). Post-Phase-3 this is the pre-aggregation survivor count. Opaque accessor so
    /// external code (tests) needs neither the field nor the `pub(crate)` `LineKey` key type.
    pub fn tick_count(&self) -> usize {
        self.lines_with_hits.len()
    }

    /// Iterates surviving ticks as `(line, column, execution_time)` — the emit-relevant tuple
    /// — without exposing the internal `LineKey`-keyed map or `PositionTick` type.
    pub fn tick_values(&self) -> impl Iterator<Item = (i32, i32, i64)> + '_ {
        self.lines_with_hits
            .values()
            .map(|(_, t)| (t.line, t.column, t.execution_time))
    }

    /// algorithm spec §7.3 `node.PositionTicks`: the surviving `lines_with_hits` entries whose owner
    /// `ExecutionIndex == ix`, as `(line, column, ticks, execution_time)`, sorted ascending by
    /// `line`. Opaque accessor — the emit walk needs this per-owner filter without reaching
    /// into the internal `LineKey`-keyed map or `PositionTick` type.
    pub fn ticks_for_owner(&self, ix: u32) -> Vec<(i32, i32, i32, i64)> {
        let mut v: Vec<(i32, i32, i32, i64)> = self
            .lines_with_hits
            .values()
            .filter(|(owner, _)| *owner == ix)
            .map(|(_, t)| (t.line, t.column, t.ticks, t.execution_time))
            .collect();
        v.sort_by_key(|t| t.0);
        v
    }

    /// Resolve a frame by its `ExecutionIndex` (via `by_index`). Panics if absent — callers
    /// pass indexes that are known-live (survivors, parent pointers). Merged-away frames stay
    /// resolvable (they are only logically removed), so this works throughout Phase 4.
    pub fn frame(&self, ix: u32) -> &CallFrameDef {
        &self.frames[self.by_index[&ix]]
    }

    /// `ix`'s ORIGINAL `ParentCallerFrameDefinition` index (algorithm spec §7.6). This is
    /// `CallFrameDef::parent`, which Phase 4 aggregation never mutates (it only ever adjusts
    /// `called_frame_indexes`/`hit_count`/`aggregated_execution_time`) — so this always walks
    /// the pre-aggregation ancestry, including merged-away frames (resolvable via `by_index`).
    pub fn parent_of(&self, ix: u32) -> Option<u32> {
        self.frame(ix).parent
    }

    /// Mutable `frame`. Borrow discipline (see `v8_aggregate`): never hold two of these into
    /// `self.frames` at once — copy scalars out, mutate, write back.
    pub(crate) fn frame_mut(&mut self, ix: u32) -> &mut CallFrameDef {
        let slot = self.by_index[&ix];
        &mut self.frames[slot]
    }

    /// Fallible `frame` — for lookups that may target an index not present in `by_index`.
    pub(crate) fn try_frame(&self, ix: u32) -> Option<&CallFrameDef> {
        self.by_index.get(&ix).map(|&slot| &self.frames[slot])
    }

    /// All frames' `ExecutionIndex`es, ascending — the §6.2 merge-pass driver order. Includes
    /// every frame; the merge pass skips ones already folded via the local `merged` set.
    pub(crate) fn frames_execution_indexes_ascending(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.frames.iter().map(|f| f.execution_index).collect();
        v.sort_unstable();
        v
    }

    /// Surviving frames' `ExecutionIndex`es (frames minus `removed`), ascending. Post-Phase-4
    /// this is the 706-node set the emit walk iterates.
    pub fn survivor_indexes(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .frames
            .iter()
            .map(|f| f.execution_index)
            .filter(|ix| !self.removed.contains(ix))
            .collect();
        v.sort_unstable();
        v
    }

    /// §6.2b logical removal: mark `ix` as merged away. It stays in `frames`/`by_index`
    /// (resolvable) but drops out of `survivor_indexes()`.
    pub(crate) fn remove_from_survivors(&mut self, ix: u32) {
        self.removed.insert(ix);
    }

    /// Caller-position-tick lookup (algorithm spec §5): the mutable
    /// position-tick of `frame_ix`'s **caller statement**. `None` when the frame has no
    /// caller line (a root), its `hit_count <= 0`, or the caller line has no surviving tick.
    /// Shared by Phase 3 (§5), §6.7 ancestry propagation, and §6.8 child-sum adjustment.
    pub(crate) fn caller_position_tick_mut(&mut self, frame_ix: u32) -> Option<&mut PositionTick> {
        let &slot = self.by_index.get(&frame_ix)?;
        if self.frames[slot].hit_count <= 0 {
            return None;
        }
        let caller_line = self.side.get(frame_ix as usize).and_then(|(cl, _)| *cl)?;
        let key = LineKey::of(&caller_line);
        self.lines_with_hits.get_mut(&key).map(|(_, tick)| tick)
    }
}

/// Rebuilds the V8 working graph from the IR. One `CallFrameDef` per invocation whose
/// `object.is_some()`; `lines_with_hits` is seeded straight from each invocation's `lines`
/// (algorithm spec §4.6 owner + first-span already resolved by `build_ir`, Task 5 — each `LineHit`
/// lives on exactly one invocation, so no first-writer-wins merge is needed here).
pub fn build_state(profile: &Profile) -> V8State {
    let n = profile.invocations.len();

    let mut side: Vec<(Option<LineDef>, Option<LineDef>)> = Vec::with_capacity(n);
    let mut contexts_incomplete: Vec<bool> = Vec::with_capacity(n);
    for inv in &profile.invocations {
        side.push((inv.caller_line, inv.called_line));
        contexts_incomplete.push(inv.is_incomplete);
    }

    let mut frames: Vec<CallFrameDef> = Vec::new();
    let mut by_index: HashMap<u32, usize> = HashMap::new();
    let mut lines_with_hits: HashMap<LineKey, (u32, PositionTick)> = HashMap::new();

    for inv in &profile.invocations {
        let Some(obj) = inv.object else { continue };

        let aggregated_execution_time =
            inv.span.map(|s| s.end_ticks - s.start_ticks).unwrap_or(0);
        by_index.insert(inv.index, frames.len());
        frames.push(CallFrameDef {
            line_definition: inv.called_line,
            parent: inv.parent_ix,
            execution_index: inv.index,
            called_frame_indexes: inv.child_ixs.clone(),
            hit_count: 1,
            aggregated_execution_time,
        });

        for hit in &inv.lines {
            let ld = LineDef { id: obj, span: hit.span };
            let key = LineKey::of(&ld);
            lines_with_hits.insert(
                key,
                (
                    inv.index,
                    PositionTick {
                        line: hit.span.from.line as i32,
                        column: hit.span.from.column as i32,
                        ticks: hit.hit_count,
                        execution_time: 0,
                    },
                ),
            );
        }
    }

    V8State {
        frames,
        by_index,
        lines_with_hits,
        side,
        contexts_incomplete,
        removed: HashSet::new(),
    }
}

/// algorithm spec §5 Phase 3 — UpdateStatementExecutionTimeWithFrameExecutionTime. Two steps, in
/// order: (1) attribute every surviving frame's `aggregated_execution_time` into its
/// caller's statement tick; (2) prune every tick whose `execution_time` is still 0.
pub fn phase3_attribute_and_prune(state: &mut V8State) {
    // Step 1 — GetCallerPositionTickInfo + accumulate, for EVERY frame. Uses the shared
    // caller-tick helper (also used by §6.7 / §6.8). Iterate by index so the helper (which
    // needs `&mut self`) can run without holding a `&self.frames` iterator open.
    let order: Vec<u32> = state.frames.iter().map(|f| f.execution_index).collect();
    for ix in order {
        let exec_time = state.frame(ix).aggregated_execution_time;
        if let Some(tick) = state.caller_position_tick_mut(ix) {
            tick.execution_time += exec_time;
        }
    }

    // Step 2 — prune zero-time ticks. Must run strictly after step 1 finishes for ALL frames.
    state.lines_with_hits.retain(|_, (_, tick)| tick.execution_time != 0);
}

/// algorithm spec §6 Phase 4 — AggregateFrames. Collapse repeated invocations of the same call
/// (same parent + same called line) into one representative, then repair the parent/child
/// links that collapsing broke. Implemented in `super::v8_aggregate`.
pub fn aggregate_frames(state: &mut V8State) {
    crate::emit::v8_aggregate::aggregate_frames(state);
}

/// algorithm spec §7.6 `CreateCallFrameIdentifier` — walks `ix`'s ORIGINAL parent chain (via
/// `state.parent_of`, which reads the never-mutated `CallFrameDef::parent`, so this reaches
/// merged-away ancestors too — they stay resolvable via `by_index`), builds
/// `[frame_hash, parent_hash, ...]` (nearest ancestor first), and folds with
/// `hash::frame_identifier`. Each frame's own hash is `frame_get_hash_code(line_hash,
/// execution_index)`, where `line_hash` is `line_get_hash_code(line.from.line,
/// line.id.get_hash_code())` if the frame has a `line_definition`, else `0`.
///
/// NOTE: this is `pub` (not `pub(crate)` as the task brief's interface prose suggested) —
/// `tests/frame_id.rs` is a separate integration-test crate and can only see `pub` items;
/// `pub(crate)` would make the brute-force test fail to compile.
pub fn frame_identifier_for(state: &V8State, ix: u32, form: CombineForm) -> i32 {
    fn own_hash(state: &V8State, ix: u32, form: CombineForm) -> i32 {
        let f = state.frame(ix);
        let line_hash = match f.line_definition {
            Some(ld) => hash::line_get_hash_code(form, ld.span.from.line, ld.id.get_hash_code()),
            None => 0,
        };
        hash::frame_get_hash_code(form, line_hash, f.execution_index as i32)
    }

    let mut chain: Vec<i32> = vec![own_hash(state, ix, form)];
    let mut cur = state.parent_of(ix);
    while let Some(p) = cur {
        chain.push(own_hash(state, p, form));
        cur = state.parent_of(p);
    }
    hash::frame_identifier(form, &chain)
}

/// V8 (`.alcpuprofile`) emitter — wires phases 3/4 (already implemented above /
/// `v8_aggregate`) into phase 5 (algorithm spec §7): build one `CpuProfileNode` per surviving frame,
/// the three parallel scalar arrays, and serialize.
pub struct V8Emitter;

impl Emitter for V8Emitter {
    fn emit(&self, profile: &Profile, opts: &EmitOptions) -> Result<Vec<u8>> {
        let mut state = build_state(profile);
        phase3_attribute_and_prune(&mut state);
        aggregate_frames(&mut state);

        // §7.2: survivors are already ascending ExecutionIndex order.
        let survivors = state.survivor_indexes();
        let n = survivors.len();
        let mut nodes = Vec::with_capacity(n);
        let mut samples = Vec::with_capacity(n);
        let mut time_deltas = Vec::with_capacity(n);
        let mut sample_execution_times: Vec<i64> = Vec::with_capacity(n);

        for &ix in &survivors {
            let f = state.frame(ix);
            let exec_time = f.aggregated_execution_time;
            let hit_count = f.hit_count;
            let children: Vec<i32> = {
                let mut c: Vec<i32> = f.called_frame_indexes.iter().map(|&c| c as i32 + 1).collect();
                c.sort_unstable();
                c
            };

            samples.push(ix as i32 + 1);
            let prev = sample_execution_times.last().copied();
            sample_execution_times.push(exec_time);
            time_deltas.push(match prev {
                None => exec_time,
                Some(p) => (exec_time - p).abs(),
            });

            let inv = &profile.invocations[ix as usize];
            let frame_identifier = frame_identifier_for(&state, ix, hash::LOCKED_FORM);
            let position_ticks = state
                .ticks_for_owner(ix)
                .into_iter()
                .map(|(line, column, ticks, execution_time)| v8_json::PositionTickOut {
                    line,
                    column,
                    ticks,
                    execution_time,
                })
                .collect();
            let (start_time, end_time) = match inv.span {
                Some(s) => (s.start_ticks, s.end_ticks),
                None => (0, 0),
            };

            nodes.push(v8_json::CpuProfileNode {
                id: ix as i32 + 1,
                call_frame: build_call_frame(inv, opts),
                hit_count,
                children,
                position_ticks,
                declaring_application: build_declaring_application(inv),
                application_definition: build_application_definition(inv),
                is_incomplete_measurement: inv.is_incomplete,
                is_builtin_code_unit_call: inv.is_builtin,
                frame_identifier,
                start_time,
                end_time,
            });
        }

        // §7.8: sort nodes ascending by id (already true — survivors is ascending — but sort
        // to be safe per the brief).
        nodes.sort_by_key(|n| n.id);

        let cpu = v8_json::CpuProfile {
            nodes,
            start_time: profile.start_time_ticks,
            end_time: profile.end_time_ticks,
            samples,
            time_deltas,
            sample_execution_times,
        };

        serde_json::to_vec(&cpu).map_err(|e| Error::Emit(e.to_string()))
    }
}

/// §7.4 `CreateCpuProfileCallFrame`. `thisFrame` absent => the all-default empty call frame.
/// `scriptId`/`lineNumber`/`columnNumber` come off `thisFrame`; `url` is built from the
/// invocation's own object id (`mc.Id`, §7.7), which is guaranteed `Some` for every survivor
/// (frames are only ever built from invocations with `object.is_some()` — see `build_state`).
fn build_call_frame(inv: &crate::ir::Invocation, opts: &EmitOptions) -> v8_json::CpuProfileCallFrame {
    let Some(tf) = &inv.this_frame else {
        // §7.4 `EmptyCpuProfileCallFrame`: null strings (=> omitted keys), zero line/column.
        return v8_json::CpuProfileCallFrame {
            function_name: None,
            script_id: None,
            url: None,
            line_number: 0,
            column_number: 0,
        };
    };

    let (line_number, column_number) = tf
        .statement_span
        .map(|s| (s.from.line as i32, s.from.column as i32))
        .unwrap_or((0, 0));
    let script_id = tf.id.map(|id| id.to_script_id()).unwrap_or_default();
    let url = match inv.object {
        Some(obj) => {
            let app_id = inv
                .app
                .as_ref()
                .map(|a| a.app_id.as_str())
                .filter(|s| !s.is_empty());
            v8_json::build_url(obj, app_id, opts.project_name.as_deref())
        }
        None => String::new(),
    };

    // Normal frame: strings are `Some(...)` (present), so a genuinely empty `method_name` still
    // serializes as a present `""` — unchanged from before the `Option` wrapping.
    v8_json::CpuProfileCallFrame {
        function_name: Some(tf.method_name.clone()),
        script_id: Some(script_id),
        url: Some(url),
        line_number,
        column_number,
    }
}

/// §7.5 `GetOrAddApplicationDefinition`. `None` object => the `Default` sentinel
/// (`objectId: -1, objectType: "", objectName: ""`).
fn build_application_definition(inv: &crate::ir::Invocation) -> v8_json::ApplicationDefinitionOut {
    match inv.object {
        None => v8_json::ApplicationDefinitionOut {
            object_type: String::new(),
            object_name: String::new(),
            object_id: -1,
        },
        Some(obj) => {
            let object_name = inv
                .this_frame
                .as_ref()
                .map(|tf| v8_json::quote_identifier_if_needed(&tf.object_name))
                .unwrap_or_default();
            v8_json::ApplicationDefinitionOut {
                object_type: obj.object_type.name().to_string(),
                object_name,
                object_id: obj.number,
            }
        }
    }
}

/// §7.5 `GetOrAddDeclaringApplication`. `None` app => the `Default` sentinel
/// (`appVersion: "0.0.0.0"`, everything else empty).
fn build_declaring_application(inv: &crate::ir::Invocation) -> v8_json::DeclaringApplicationOut {
    match &inv.app {
        None => v8_json::DeclaringApplicationOut {
            app_id: String::new(),
            app_name: String::new(),
            app_publisher: String::new(),
            app_version: "0.0.0.0".to_string(),
        },
        Some(app) => v8_json::DeclaringApplicationOut {
            app_id: app.app_id.clone(),
            app_name: app.app_name.clone(),
            app_publisher: app.app_publisher.clone(),
            app_version: app.app_version.clone(),
        },
    }
}

