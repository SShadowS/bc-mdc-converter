// Decodes a single `.mdc` (MethodContext) or `version` FlatBuffers payload into typed
// Rust structs. Field slot map: .mdc schema spec §3/§4.

use crate::flatbuffers::Table;
use crate::mdc::types::*;
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct StackFrame {
    pub method_name: String,
    pub id: Option<ApplicationObjectId>,
    pub object_name: String,
    pub statement_span: Option<SourceSpan>,
}
#[derive(Clone, Debug)]
pub struct ExceptionInfo {
    pub message: String,
    pub line: u16,
}
#[derive(Clone, Debug)]
pub struct MethodContext {
    pub method_name: String,
    pub id: Option<ApplicationObjectId>,
    pub app: Option<DeclaringApplication>,
    pub execution_span: Option<(i64, i64)>, // (start, end) ticks
    pub is_builtin: bool,
    pub is_incomplete: bool,
    pub frames: Vec<StackFrame>,
    pub line_contexts: Vec<SourceSpan>, // statement_span per LineContext, in order
    pub exception: Option<ExceptionInfo>,
}

fn read_object_id(t: &Table) -> Option<ApplicationObjectId> {
    // ApplicationObjectId table: number id0, type id1 (i8, default 0)
    let number = t.scalar_i32(0, 0);
    let ty = ObjectType::from_i8(t.scalar_i8(1, 0));
    // Present iff the id table itself was present (caller decides via table()).
    Some(ApplicationObjectId { number, object_type: ty })
}
fn read_span_inline(parent: &Table, id: u16) -> Option<SourceSpan> {
    // SourceSpan inline struct: to@+0, from@+4; each SourcePosition line@+0 col@+2 (u16)
    let to = SourcePosition { line: parent.struct_u16(id, 0)?, column: parent.struct_u16(id, 2)? };
    let from = SourcePosition { line: parent.struct_u16(id, 4)?, column: parent.struct_u16(id, 6)? };
    Some(SourceSpan { from, to })
}
fn read_stack_frame(t: &Table) -> StackFrame {
    StackFrame {
        method_name: t.string(0).unwrap_or_default().to_string(),
        id: t.table(1).and_then(|idt| read_object_id(&idt)),
        object_name: t.string(2).unwrap_or_default().to_string(),
        statement_span: read_span_inline(t, 3),
    }
}

pub fn decode_method_context(bytes: &[u8]) -> Result<MethodContext> {
    if bytes.is_empty() {
        return Err(Error::Decode("empty .mdc".into()));
    }
    let t = Table::root(bytes);
    let id = t.table(0).and_then(|idt| read_object_id(&idt));
    let method_name = t.string(1).unwrap_or_default().to_string();
    let mut frames = Vec::new();
    for i in 0..t.vector_len(3) {
        if let Some(ft) = t.vector_elem_table(3, i) {
            frames.push(read_stack_frame(&ft));
        }
    }
    let mut line_contexts = Vec::new();
    for i in 0..t.vector_len(4) {
        if let Some(lc) = t.vector_elem_table(4, i) {
            if let Some(span) = read_span_inline(&lc, 1) {
                line_contexts.push(span);
            }
            // LineContexts with no span are simply skipped (algorithm spec §4.4.1).
        }
    }
    let execution_span = t
        .field_offset(5)
        .map(|_| (t.struct_i64(5, 0).unwrap(), t.struct_i64(5, 8).unwrap()));
    let app = t.table(6).map(|a| DeclaringApplication {
        app_name: a.string(0).unwrap_or_default().to_string(),
        app_publisher: a.string(1).unwrap_or_default().to_string(),
        app_version: a.string(2).unwrap_or_default().to_string(),
        app_id: a.string(3).unwrap_or_default().to_string(),
    });
    let exception = t.table(2).map(|t_exc| ExceptionInfo {
        message: t_exc.string(0).unwrap_or_default().to_string(),
        line: read_span_inline(&t_exc, 2)
            .map(|s| s.from.line)
            .unwrap_or_else(|| t_exc.scalar_i32(1, 0) as u16),
    });
    Ok(MethodContext {
        method_name,
        id,
        app,
        execution_span,
        is_builtin: t.scalar_i8(7, 0) == 1,
        is_incomplete: t.scalar_i8(8, 0) == 1,
        frames,
        line_contexts,
        exception,
    })
}

pub fn decode_version(bytes: &[u8]) -> (i32, i32, i32, i32) {
    let t = Table::root(bytes);
    (t.scalar_i32(0, 3), t.scalar_i32(1, 0), t.scalar_i32(2, 0), t.scalar_i32(3, 0))
}
