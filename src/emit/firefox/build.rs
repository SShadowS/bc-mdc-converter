// IR -> Firefox v54 thread tables (Firefox emitter spec §2.1-§2.4). This is the
// deduped call-tree data structure the flame graph / call tree / stack chart all key off:
// a string interner, categories/subcategories, and the func/frame/stack/resource tables.
//
// `pub` throughout (not `pub(crate)`) per the Global Constraint "test visibility convention"
// (Firefox emitter plan): `tests/firefox.rs` (an
// external integration crate) calls `build_tables` directly and reads the result's fields,
// ahead of `FirefoxEmitter::emit` being wired end-to-end at FT5.

use std::collections::HashMap;

use crate::emit::al_url::{al_object_descriptor, build_url};
use crate::emit::firefox::model::{
    marker_schema, Category, FrameTable, FuncTable, Lib, Meta, ResourceTable, StackTable,
    CATEGORIES, SUBCATEGORIES,
};
use crate::emit::firefox::{tick_to_ms, wire_line_to_display};
use crate::ir::{LineKey, Profile};
use crate::mdc::types::{ApplicationObjectId, DeclaringApplication, ObjectType};
use crate::EmitOptions;

/// First-seen string interner -> `stringArray` (Firefox emitter spec §5.3). A func name shared by 500
/// frames is one entry. Insertion order = index order, so output bytes are deterministic
/// across runs (Firefox emitter spec §7.4) — never iterate `by_str` to produce output.
#[derive(Default, Debug)]
pub struct Interner {
    pub by_str: HashMap<String, u32>,
    pub strings: Vec<String>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&ix) = self.by_str.get(s) {
            return ix;
        }
        let ix = self.strings.len() as u32;
        self.strings.push(s.to_string());
        self.by_str.insert(s.to_string(), ix);
        ix
    }
}

/// Firefox emitter spec §2.1: object-type -> category index. `is_builtin` only changes the outcome for
/// `CodeUnit` (a built-in codeunit call gets its own category, 7, instead of 1). No valid id
/// (`obj.is_none()`) and `ObjectType::System` both land in the System/platform bucket (8).
pub fn category_for(obj: Option<ApplicationObjectId>, is_builtin: bool) -> u32 {
    let Some(id) = obj else { return 8 };
    match id.object_type {
        ObjectType::CodeUnit => {
            if is_builtin {
                7
            } else {
                1
            }
        }
        ObjectType::Table => 2,
        ObjectType::Page => 3,
        ObjectType::Report => 4,
        ObjectType::Query => 5,
        ObjectType::XmlPort => 6,
        ObjectType::System => 8,
        _ => 0,
    }
}

/// Firefox emitter spec §2.2: method-kind -> subcategory index (the shared vocabulary across all
/// categories, `model::SUBCATEGORIES`). `is_builtin` wins over the name pattern; an unknown
/// (empty) method name is `Other`, not `Procedure`.
pub fn subcategory_for(method_name: &str, is_builtin: bool) -> u32 {
    if is_builtin {
        return 3; // Built-in
    }
    if method_name.is_empty() {
        return 0; // Other / unknown
    }
    if is_trigger_name(method_name) {
        1 // Trigger
    } else {
        2 // Procedure
    }
}

/// AL trigger naming convention: `On` + a PascalCase word (`OnRun`, `OnInsert`, `OnModify`,
/// `OnValidate`, `OnAfterGetRecord`, `OnOpenPage`, `OnAction`, `OnPreReport`, ... Firefox emitter spec §2.2).
/// A short allowlist covers the common ones exactly; the generic `On<Uppercase>` prefix check
/// catches the rest without matching a name that merely happens to start lowercase after "On".
fn is_trigger_name(name: &str) -> bool {
    const KNOWN: &[&str] = &[
        "OnRun",
        "OnInsert",
        "OnModify",
        "OnDelete",
        "OnRename",
        "OnValidate",
        "OnLookup",
        "OnAfterGetRecord",
        "OnBeforeGetRecord",
        "OnOpenPage",
        "OnClosePage",
        "OnAction",
        "OnPreReport",
        "OnPostReport",
        "OnInitReport",
        "OnQueryClosePage",
    ];
    if KNOWN.contains(&name) {
        return true;
    }
    let bytes = name.as_bytes();
    name.starts_with("On") && bytes.len() > 2 && bytes[2].is_ascii_uppercase()
}

/// Everything `build_tables` produces from one `Profile` (Firefox emitter spec §2.3/§2.4).
#[derive(Debug)]
pub struct BuiltTables {
    pub interner: Interner,
    pub funcs: FuncTable,
    pub frames: FrameTable,
    pub stacks: StackTable,
    pub resources: ResourceTable,
    pub libs: Vec<Lib>,
    /// `stack_node_of_invocation[ix]` = the `stackTable` row index for invocation `ix`, or
    /// `None` if that invocation created no frame (`object.is_none()`).
    pub stack_node_of_invocation: Vec<Option<u32>>,
}

/// Resource (declaring-app) dedup key: `app_id` when non-empty, else `app_name` (Firefox emitter spec §2.3).
fn resource_key(app: &DeclaringApplication) -> String {
    if !app.app_id.is_empty() {
        app.app_id.clone()
    } else {
        app.app_name.clone()
    }
}

/// Builds the deduped func/frame/stack/resource tables from the IR (Firefox emitter spec §2.3/§2.4).
/// Thin wrapper computing the temporal nesting via the boundary sweep, then delegating to
/// `build_tables_with_parents`. Call sites that already have a `SweepResult` (e.g. `emit`) should
/// call `build_tables_with_parents` directly to avoid running the sweep twice.
pub fn build_tables(profile: &Profile, opts: &EmitOptions) -> BuiltTables {
    let (temporal_parent, _self_ticks) = crate::emit::firefox::sweep::temporal_nesting(profile);
    build_tables_with_parents(profile, opts, &temporal_parent)
}

/// Builds the deduped func/frame/stack/resource tables from the IR (Firefox emitter spec §2.3/§2.4), using the
/// supplied `temporal_parent` (from the boundary sweep, `sweep::run_sweep`/`temporal_nesting`) to
/// set each `stackTable` node's `prefix`. Invocations are processed in ascending `index` order
/// (their natural order in `profile.invocations`), which is what makes `stackTable` topologically
/// ordered: a parent's stack node is always created (and therefore has a lower index) before its
/// children's — the temporal parent (the actual caller) always has a lower execution index than its
/// callee (algorithm spec §4.2: callers precede callees in the `.mdc` ordinal), so the prefix index is
/// always strictly lower than its own node index.
pub fn build_tables_with_parents(
    profile: &Profile,
    opts: &EmitOptions,
    temporal_parent: &[Option<u32>],
) -> BuiltTables {
    let mut interner = Interner::default();
    let mut funcs = FuncTable::default();
    let mut frames = FrameTable::default();
    let mut stacks = StackTable::default();
    let mut resources = ResourceTable::default();
    let mut libs: Vec<Lib> = Vec::new();

    let mut resource_by_key: HashMap<String, u32> = HashMap::new();
    let mut func_by_key: HashMap<(ApplicationObjectId, String), u32> = HashMap::new();
    let mut frame_by_key: HashMap<(u32, Option<LineKey>), u32> = HashMap::new();

    let mut stack_node_of_invocation: Vec<Option<u32>> = vec![None; profile.invocations.len()];

    for inv in &profile.invocations {
        let Some(obj) = inv.object else { continue }; // no valid id -> no frame/stack node

        let method_name = inv.this_frame.as_ref().map(|f| f.method_name.as_str()).unwrap_or("");
        let object_name = inv.this_frame.as_ref().map(|f| f.object_name.as_str()).unwrap_or("");

        // --- resourceTable + libs: dedup by declaring app (Firefox emitter spec §2.3) ---
        let has_app = matches!(&inv.app, Some(a) if !a.app_id.is_empty() || !a.app_name.is_empty());
        let resource_ix: i32 = if has_app {
            let app = inv.app.as_ref().unwrap();
            let key = resource_key(app);
            let ix = *resource_by_key.entry(key).or_insert_with(|| {
                let idx = resources.name.len() as u32;
                let name_ix = interner.intern(&app.app_name);
                let lib_ix = libs.len() as u32;
                libs.push(Lib {
                    arch: String::new(),
                    name: app.app_name.clone(),
                    path: String::new(),
                    debug_name: app.app_name.clone(),
                    debug_path: String::new(),
                    breakpad_id: if !app.app_id.is_empty() {
                        app.app_id.clone()
                    } else {
                        "0".to_string()
                    },
                    code_id: None,
                });
                resources.lib.push(lib_ix);
                resources.name.push(name_ix);
                resources.host.push(None);
                resources.resource_type.push(1); // ResourceType::Library
                resources.length = resources.name.len();
                idx
            });
            ix as i32
        } else {
            -1
        };

        // --- funcTable: dedup by (object_type, object_id, method_name) (Firefox emitter spec §2.4) ---
        let func_key = (obj, method_name.to_string());
        let func_ix = *func_by_key.entry(func_key).or_insert_with(|| {
            let idx = funcs.name.len() as u32;

            let name = format!(
                "{}.{}",
                al_object_descriptor(obj.object_type.firefox_name(), obj.number, object_name),
                method_name,
            );
            let name_ix = interner.intern(&name);

            let app_id =
                inv.app.as_ref().map(|a| a.app_id.as_str()).filter(|s| !s.is_empty());
            let url = build_url(obj, app_id, opts.project_name.as_deref());
            let file_name_ix = interner.intern(&url);

            let (line, column) = inv
                .called_line
                .map(|ld| (wire_line_to_display(ld.span.from.line), ld.span.from.column as u32))
                .unwrap_or((0, 0));

            funcs.name.push(name_ix);
            funcs.is_js.push(false);
            funcs.relevant_for_js.push(false);
            funcs.resource.push(resource_ix);
            funcs.file_name.push(file_name_ix);
            funcs.line_number.push(line);
            funcs.column_number.push(column);
            funcs.length = funcs.name.len();
            idx
        });

        // --- frameTable: dedup by (func, called_line) (Firefox emitter spec §2.4) ---
        let line_key = inv.called_line.map(|ld| LineKey::of(&ld));
        let frame_key = (func_ix, line_key);
        let frame_ix = *frame_by_key.entry(frame_key).or_insert_with(|| {
            let idx = frames.func.len() as u32;

            let category = category_for(inv.object, inv.is_builtin);
            let subcategory = subcategory_for(method_name, inv.is_builtin);
            let (line, column) = inv
                .called_line
                .map(|ld| (wire_line_to_display(ld.span.from.line), ld.span.from.column as u32))
                .unwrap_or((0, 0));

            frames.func.push(func_ix);
            frames.address.push(-1);
            frames.inline_depth.push(0);
            frames.category.push(category);
            frames.subcategory.push(subcategory);
            frames.native_symbol.push(None);
            frames.inner_window_id.push(None);
            frames.line.push(line);
            frames.column.push(column);
            frames.length = frames.func.len();
            idx
        });

        // --- stackTable: one node per invocation, execution order == topological order ---
        let category = frames.category[frame_ix as usize];
        let subcategory = frames.subcategory[frame_ix as usize];
        // Prefix = the stack node of this invocation's TRUE temporal parent (the boundary sweep's
        // `open.last()` at ENTER), NOT the IR's V8-aggregation `parent_ix` (Firefox emitter spec §7.1). The
        // sweep's `open` holds only framed invocations, so `temporal_parent[ix]` is already the
        // nearest framed temporal ancestor and always carries a stack node; `None` => a temporal
        // root (or an invocation the sweep dropped), which is a stackTable root (`prefix = None`).
        let prefix = temporal_parent[inv.index as usize]
            .and_then(|pix| stack_node_of_invocation[pix as usize]);

        let node_ix = stacks.frame.len() as u32;
        stacks.frame.push(frame_ix);
        stacks.prefix.push(prefix);
        stacks.category.push(category);
        stacks.subcategory.push(subcategory);
        stacks.length = stacks.frame.len();

        stack_node_of_invocation[inv.index as usize] = Some(node_ix);
    }

    BuiltTables { interner, funcs, frames, stacks, resources, libs, stack_node_of_invocation }
}

/// Assembles `meta` (Firefox emitter spec §5.1). `t0` is `profile_t0(profile)`, already computed once by the
/// caller (`FirefoxEmitter::emit`) and shared with the sweep/markers passes — never recomputed
/// here. See `model::Meta`'s doc-comment for the `startTime = 0.0` deviation rationale.
pub fn build_meta(profile: &Profile, t0: i64) -> Meta {
    let categories: Vec<Category> = CATEGORIES
        .iter()
        .map(|&(name, color)| Category {
            name: name.to_string(),
            color: color.to_string(),
            subcategories: SUBCATEGORIES.iter().map(|s| s.to_string()).collect(),
        })
        .collect();

    Meta {
        version: 34,
        preprocessed_profile_version: 54,
        interval: 1,
        start_time: 0.0, // documented deviation — see `model::Meta` doc-comment
        end_time: tick_to_ms(profile.end_time_ticks, t0),
        process_type: 0,
        stackwalk: 0,
        symbolicated: true,
        // ASCII hyphen, not an em-dash: profiler.firefox.com's title rendered "—" as mojibake.
        product: format!("BC Instrumentation - {}", first_app_name(profile)),
        categories,
        marker_schema: marker_schema(),
        keep_profile_thread_order: true,
    }
}

/// First non-empty declaring-app name in execution order, or `"AL"` if no invocation carries one
/// (Firefox emitter spec §5.1 `product`). `Profile.apps` (IR architecture spec) is currently always empty — `build_ir`
/// never populates it (a Plan 1 leftover) — so this reads the per-invocation
/// `DeclaringApplication` instead, which IS populated.
fn first_app_name(profile: &Profile) -> String {
    for inv in &profile.invocations {
        if let Some(app) = &inv.app {
            if !app.app_name.is_empty() {
                return app.app_name.clone();
            }
        }
    }
    "AL".to_string()
}
