// Builds the IR `Profile` from decoded MDC method contexts.
// Ports algorithm spec §4 (Phases 1-2: ExtractMethodContexts +
// CreateCallFrameDefinitions) — flat, execution-ordered `MethodContext`s in,
// a linked call tree (parent_ix / child_ixs) plus raw per-line hit counts out.
//
// `LineKey`, `line_opt_eq`, and `line_def_from_frame` live in `crate::ir` so both this
// module (Phase 2) and `emit::v8` (Phase 3) share ONE definition.

use std::collections::{HashMap, HashSet};
use crate::ir::*;
use crate::mdc::schema::MethodContext;

/// GetCalledLine = line definition from `frames[0]` (the executed frame).
fn get_called_line(mc: &MethodContext) -> Option<LineDef> {
    mc.frames.first().and_then(line_def_from_frame)
}

/// GetCallerLine, strict: None when `frames.len() < 2` (a single-frame context is a root).
fn get_caller_line(mc: &MethodContext) -> Option<LineDef> {
    if mc.frames.len() > 1 { line_def_from_frame(&mc.frames[1]) } else { None }
}

pub fn build_profile(contexts: &[MethodContext]) -> Profile {
    let n = contexts.len();

    // --- ExtractMethodContexts side dict (§4.1) ---
    let side: Vec<(Option<LineDef>, Option<LineDef>)> = contexts
        .iter()
        .map(|mc| (get_caller_line(mc), get_called_line(mc)))
        .collect();

    // --- CreateCallFrameDefinitions (§4.2) ---
    let mut lines_with_indexes: HashMap<LineKey, usize> = HashMap::new();
    // per-invocation working output:
    let mut parent_ix: Vec<Option<u32>> = vec![None; n];
    let mut child_ixs: Vec<Vec<u32>> = vec![Vec::new(); n];
    // raw line hits: key -> (first index, first-encountered span, hit_count)
    let mut lines_with_hits: HashMap<LineKey, (usize, crate::mdc::types::SourceSpan, i32)> =
        HashMap::new();

    for i in 0..n {
        let obj = match contexts[i].id {
            Some(id) => id,
            None => continue, // §4.2.1: no valid id -> skip frame (still consumes index i)
        };
        let (caller_line, _called_line) = side[i];

        // parent = GetCallerCallFrameDefinition (§4.2.3 / :392-403)
        parent_ix[i] = match caller_line {
            None => None,
            Some(cl) => lines_with_indexes.get(&LineKey::of(&cl)).map(|&ix| ix as u32),
        };

        // window: startIndex = min(i+1, n-1); nextSibling = GetNextSiblingIndex (§4.3)
        let start_index = (i + 1).min(n - 1);
        let next_sibling = get_next_sibling_index(&side, caller_line, start_index, n);

        // TraverseLinesAndCreateCalledFrameIndexes (§4.4)
        let mut seen: HashSet<LineKey> = HashSet::new();
        let mut called: HashSet<usize> = HashSet::new();
        for span in &contexts[i].line_contexts {
            let ld = LineDef { id: obj, span: *span };
            let key = LineKey::of(&ld);
            if seen.insert(key) {
                lines_with_indexes.entry(key).or_insert(i); // first-writer-wins (§4.4.3)
                for c in get_called_indexes_for_line(&side, i, next_sibling, &ld) {
                    called.insert(c);
                }
            }
            // ALWAYS update hit count (§4.4.4 / §4.6)
            lines_with_hits
                .entry(key)
                .and_modify(|e| e.2 += 1)
                .or_insert((i, *span, 1));
        }
        let mut cf: Vec<u32> = called.into_iter().map(|x| x as u32).collect();
        cf.sort_unstable();
        child_ixs[i] = cf;
    }

    // --- Assemble IR invocations ---
    // attach raw line hits to their first-index owner
    let mut per_index_lines: Vec<Vec<LineHit>> = vec![Vec::new(); n];
    // deterministic order: emit ascending by from.line then column (final V8 sorts by line anyway)
    let mut hits: Vec<(&LineKey, &(usize, crate::mdc::types::SourceSpan, i32))> =
        lines_with_hits.iter().collect();
    hits.sort_by_key(|(_, v)| (v.1.from.line, v.1.from.column));
    for (_, (owner, span, hc)) in hits {
        per_index_lines[*owner].push(LineHit { span: *span, hit_count: *hc });
    }

    let mut invocations = Vec::with_capacity(n);
    for (i, mc) in contexts.iter().enumerate() {
        let this_frame = mc.frames.first().map(|f| FrameInfo {
            method_name: f.method_name.clone(),
            id: f.id,
            object_name: f.object_name.clone(),
            statement_span: f.statement_span,
        });
        invocations.push(Invocation {
            index: i as u32,
            object: mc.id,
            this_frame,
            app: mc.app.clone(),
            span: mc.execution_span.map(|(s, e)| Span { start_ticks: s, end_ticks: e }),
            is_builtin: mc.is_builtin,
            is_incomplete: mc.is_incomplete,
            parent_ix: parent_ix[i],
            child_ixs: std::mem::take(&mut child_ixs[i]),
            caller_line: side[i].0,
            called_line: side[i].1,
            lines: std::mem::take(&mut per_index_lines[i]),
            exception: mc.exception.as_ref().map(|e| Exception { message: e.message.clone(), line: e.line }),
            sql: Vec::new(),
        });
    }

    // profile time sourcing (§7.1): ordinal-0 start, last-ordinal end.
    let start_time_ticks =
        invocations.first().and_then(|i| i.span).map(|s| s.start_ticks).unwrap_or(0);
    let end_time_ticks = invocations.last().and_then(|i| i.span).map(|s| s.end_ticks).unwrap_or(0);

    Profile { start_time_ticks, end_time_ticks, invocations, apps: Vec::new(), capture: crate::ir::CaptureMeta::default() }
}

/// §4.3 — first k>=start where side[k].callerLine == callerLine; else n-1.
fn get_next_sibling_index(
    side: &[(Option<LineDef>, Option<LineDef>)],
    caller_line: Option<LineDef>,
    start: usize,
    n: usize,
) -> usize {
    for (k, s) in side.iter().enumerate().take(n).skip(start) {
        if line_opt_eq(s.0, caller_line) {
            return k;
        }
    }
    n - 1
}

/// §4.5 — inclusive [start..=end]; prevCalled updated on EVERY hit; add on caller match AND called != prev.
fn get_called_indexes_for_line(
    side: &[(Option<LineDef>, Option<LineDef>)],
    start: usize,
    end: usize,
    line: &LineDef,
) -> Vec<usize> {
    let mut prev: Option<LineDef> = None;
    let mut out = Vec::new();
    for i in start..=end {
        let (caller, called) = side[i];
        if line_opt_eq(caller, Some(*line)) && !line_opt_eq(called, prev) {
            out.push(i);
        }
        prev = called;
    }
    out
}
