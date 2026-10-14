// Replay-side decode of one `.mdc` (MethodContext): line contexts with their variable trees and the
// exception, which the CPU-profile path skips. Slot map: .mdc schema spec (LineContext, Variable, ExceptionContext).
// Spans are kept exactly as stored (0-based lines in debugging snapshots, live Cronus29 2026-10-14).

use crate::flatbuffers::Table;
use crate::mdc::types::DeclaringApplication;
use crate::{Error, Result};

const MAX_VARIABLE_DEPTH: usize = 64; // recursion guard for hostile/corrupt input

#[derive(Clone, Debug)]
pub struct SnapVariable { pub name: String, pub type_name: String, pub value: String, pub has_children: bool, pub children: Vec<SnapVariable> }
#[derive(Clone, Debug, PartialEq)]
pub struct SnapSpan { pub from: (u16, u16), pub to: (u16, u16) }
#[derive(Clone, Debug)]
pub struct SnapLineContext { pub index: usize, pub line_number: i32, pub span: Option<SnapSpan>, pub timestamp: i64, pub variables: Vec<SnapVariable> }
#[derive(Clone, Debug)]
pub struct SnapException { pub message: String, pub line_number: i32, pub span: Option<SnapSpan> }
#[derive(Clone, Debug)]
pub struct SnapFrame { pub method_name: String, pub object_type_id: Option<i8>, pub object_id: Option<i32>, pub object_name: String, pub span: Option<SnapSpan> }
#[derive(Clone, Debug)]
pub struct SnapMethod {
    pub method_name: String,
    pub object_type_id: Option<i8>,
    pub object_id: Option<i32>,
    pub app: Option<DeclaringApplication>,
    pub is_builtin: bool,
    pub is_incomplete: bool,
    pub frames: Vec<SnapFrame>,
    pub line_contexts: Vec<SnapLineContext>,
    pub exception: Option<SnapException>,
}

fn span(t: &Table, id: u16) -> Option<SnapSpan> {
    // SourceSpan inline struct: to@+0, from@+4; SourcePosition line@+0 col@+2
    Some(SnapSpan {
        to: (t.struct_u16(id, 0)?, t.struct_u16(id, 2)?),
        from: (t.struct_u16(id, 4)?, t.struct_u16(id, 6)?),
    })
}
fn obj_id(t: &Table, id: u16) -> (Option<i8>, Option<i32>) {
    match t.table(id) {
        Some(o) => (Some(o.scalar_i8(1, 0)), Some(o.scalar_i32(0, 0))),
        None => (None, None),
    }
}
fn s(t: &Table, id: u16) -> String { t.string(id).unwrap_or_default().to_string() }

fn variable(t: &Table, depth: usize) -> SnapVariable {
    let mut children = Vec::new();
    if depth < MAX_VARIABLE_DEPTH {
        for i in 0..t.vector_len(0) {
            if let Some(c) = t.vector_elem_table(0, i) { children.push(variable(&c, depth + 1)); }
        }
    }
    SnapVariable { name: s(t, 2), type_name: s(t, 4), value: s(t, 3), has_children: t.scalar_i8(1, 0) == 1, children }
}

fn decode_unchecked(bytes: &[u8]) -> SnapMethod {
    let t = Table::root(bytes);
    let (object_type_id, object_id) = obj_id(&t, 0);
    let mut frames = Vec::new();
    for i in 0..t.vector_len(3) {
        if let Some(f) = t.vector_elem_table(3, i) {
            let (ft, fid) = obj_id(&f, 1);
            frames.push(SnapFrame { method_name: s(&f, 0), object_type_id: ft, object_id: fid, object_name: s(&f, 2), span: span(&f, 3) });
        }
    }
    let mut line_contexts = Vec::new();
    for i in 0..t.vector_len(4) {
        if let Some(lc) = t.vector_elem_table(4, i) {
            let mut variables = Vec::new();
            for k in 0..lc.vector_len(2) {
                if let Some(v) = lc.vector_elem_table(2, k) { variables.push(variable(&v, 0)); }
            }
            line_contexts.push(SnapLineContext { index: i, line_number: lc.scalar_i32(0, 0), span: span(&lc, 1), timestamp: lc.scalar_i64(3, 0), variables });
        }
    }
    let exception = t.table(2).map(|e| SnapException { message: s(&e, 0), line_number: e.scalar_i32(1, 0), span: span(&e, 2) });
    let app = t.table(6).map(|a| DeclaringApplication { app_name: s(&a, 0), app_publisher: s(&a, 1), app_version: s(&a, 2), app_id: s(&a, 3) });
    SnapMethod {
        method_name: s(&t, 1), object_type_id, object_id, app,
        is_builtin: t.scalar_i8(7, 0) == 1, is_incomplete: t.scalar_i8(8, 0) == 1,
        frames, line_contexts, exception,
    }
}

/// Never panics: the FlatBuffers reader indexes buffers directly, so malformed input is caught here.
pub fn decode_snap_method(bytes: &[u8]) -> Result<SnapMethod> {
    if bytes.len() < 8 { return Err(Error::Decode("method context too short".into())); }
    let owned = bytes.to_vec();
    std::panic::catch_unwind(move || decode_unchecked(&owned))
        .map_err(|_| Error::Decode("malformed method context".into()))
}
