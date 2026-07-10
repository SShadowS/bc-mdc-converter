// BcCall + BcException markers (Firefox emitter spec §3) — the time-ordered
// execution waterfall (Marker Chart / Marker Table) drawn on top of the flame graph built from
// the sample stream (sweep.rs) and call tree (build.rs). One `BcCall` INTERVAL marker per framed
// invocation (a span AND a valid object, §3.1); one `BcException` INSTANT marker per invocation
// that threw (§3.2), placed at the throw point (`end_ticks`). Zero `BcSql` markers are ever
// produced for the `.mdc` path (§3.3) — the schema is still registered (`model::marker_schema`)
// so a future SQL source needs no redesign.

use serde_json::json;

use crate::emit::al_url::al_object_descriptor;
use crate::emit::firefox::build::{category_for, Interner};
use crate::emit::firefox::model::MarkerTable;
use crate::emit::firefox::{profile_t0, tick_to_ms, wire_line_to_display};
use crate::ir::Profile;

/// `Exception` category index (Firefox emitter spec §2.1 idx 9) — marker-only; `frameTable`/`stackTable`
/// never point at it (`build::category_for` never returns it).
const EXCEPTION_CATEGORY: u32 = 9;

/// `<FirefoxObjectTypeName> <id> <QuotedName>` (Firefox emitter spec §3.1/§3.2), e.g. `Codeunit 80 "Sales-Post"`.
/// Delegates to `al_object_descriptor`, which already supplies the surrounding quotes ONLY for
/// non-simple names (a simple name like `Customer` stays unquoted) — never double-quote. Same
/// casing as `funcTable.name` (FT2) — `firefox_name()`, not `name()`, so `CodeUnit` prints as
/// `Codeunit`. `""` when the invocation has no valid object id.
fn object_string(
    obj: Option<crate::mdc::types::ApplicationObjectId>,
    object_name: &str,
) -> String {
    match obj {
        Some(id) => al_object_descriptor(id.object_type.firefox_name(), id.number, object_name),
        None => String::new(),
    }
}

/// Builds the `BcCall` + `BcException` marker table from the IR (Firefox emitter spec §3). `self_ms[ix]` is
/// invocation `ix`'s exact self time in ms — the sum of the boundary sweep's sub-interval weights
/// where `ix` is the leaf (§7.1); non-negative by construction. The caller supplies it from
/// `run_sweep`'s `self_ticks`. Zero `BcSql` markers are emitted.
pub fn build_markers(profile: &Profile, interner: &mut Interner, self_ms: &[f64]) -> MarkerTable {
    let t0 = profile_t0(profile);
    let mut m = MarkerTable::default();

    for inv in &profile.invocations {
        let method_name =
            inv.this_frame.as_ref().map(|f| f.method_name.as_str()).unwrap_or("");
        let object_name =
            inv.this_frame.as_ref().map(|f| f.object_name.as_str()).unwrap_or("");

        // --- BcCall: one INTERVAL marker per invocation with a span AND a valid object ---
        if let (Some(span), Some(obj)) = (inv.span, inv.object) {
            let object_str = object_string(Some(obj), object_name);
            let app_name = inv.app.as_ref().map(|a| a.app_name.as_str()).unwrap_or("");
            let line = inv
                .called_line
                .map(|ld| wire_line_to_display(ld.span.from.line))
                .unwrap_or(0);
            let duration_ms = (span.end_ticks - span.start_ticks) as f64 / 10000.0;
            let self_ms_val = self_ms.get(inv.index as usize).copied().unwrap_or(0.0);

            let data = json!({
                "type": "BcCall",
                "object": object_str,
                "objectType": obj.object_type.firefox_name(),
                "objectId": obj.number,
                "app": app_name,
                "method": method_name,
                "line": line,
                "durationMs": duration_ms,
                "selfMs": self_ms_val,
                "hitCount": 1,
                "incomplete": inv.is_incomplete,
            });

            m.name.push(interner.intern(method_name));
            m.start_time.push(Some(tick_to_ms(span.start_ticks, t0)));
            m.end_time.push(Some(tick_to_ms(span.end_ticks, t0)));
            m.phase.push(1); // INTERVAL
            m.category.push(category_for(inv.object, inv.is_builtin));
            m.data.push(data);
        }

        // --- BcException: one INSTANT marker per invocation that threw, at end_ticks ---
        if let Some(exc) = &inv.exception {
            let object_str = object_string(inv.object, object_name);
            // The throw point. `is_incomplete`/no-span invocations are not expected to carry an
            // exception in practice, but fall back to t0 rather than panicking if one does.
            let end_ticks = inv.span.map(|s| s.end_ticks).unwrap_or(t0);

            let data = json!({
                "type": "BcException",
                "message": exc.message,
                "line": wire_line_to_display(exc.line),
                "object": object_str,
                "method": method_name,
            });

            m.name.push(interner.intern("BcException"));
            m.start_time.push(Some(tick_to_ms(end_ticks, t0)));
            m.end_time.push(None); // instant
            m.phase.push(0); // INSTANT
            m.category.push(EXCEPTION_CATEGORY);
            m.data.push(data);
        }
    }

    m.length = m.name.len();
    m
}
