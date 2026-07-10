// algorithm spec §7.6 — `frameIdentifier` hash primitives.
//
// PURE module: no dependency on `emit` (the state-walking wrapper `frame_identifier_for`
// lives in `emit::v8` instead, to avoid an `emit -> hash -> emit` cycle).
//
// The one genuinely unresolved primitive in this port is Roslyn's internal `Hash.Combine`
// (its exact upstream form is unknown). Four candidate forms are implemented here; which one
// is correct is resolved empirically in `tests/frame_id.rs` by matching all 706 reference
// frames' `frameIdentifier` at once (a single node cannot disambiguate — open question #1). The
// winner is locked in `LOCKED_FORM` below once determined.

/// A candidate `Hash.Combine(acc, key)` form. `RotateXor`/`Multiply` are the two forms
/// documented in algorithm spec §7.6(A)/(B); the `*Swapped` variants swap which operand is treated
/// as the "mixed" one (`currentKey`) vs the "combined" one (`newKey`), since the argument
/// convention of the upstream `Hash.Combine` is inferred, not observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombineForm {
    RotateXor,
    Multiply,
    RotateXorSwapped,
    MultiplySwapped,
}

/// The empirically-resolved winner (see `tests/frame_id.rs::some_combine_form_matches_all_706_frame_identifiers`,
/// which checks all 706 surviving frames of `fixtures/instr.reference.alcpuprofile` against
/// each candidate). `RotateXorSwapped` is the form that makes ALL 706 match — i.e. the running
/// accumulator `acc` (not the freshly-computed `key`) is the operand that gets rotate-mixed,
/// and `key` is XORed straight in: `((acc<<5)+acc+(acc>>27)) ^ key`. Locked here so the V8
/// emitter (Task 9+) has one canonical form to use.
pub const LOCKED_FORM: CombineForm = CombineForm::RotateXorSwapped;

/// `Hash.Combine(acc, key)` — mixes `key`, combines the mixed value with `acc`. All ops are
/// i32 wrapping arithmetic; `>>` on `i32` is Rust's native arithmetic (sign-extending) shift,
/// matching C#'s `>>` on a signed `int`.
///
/// - `RotateXor`   (algorithm spec §7.6(A)): `((key << 5) + key + (key >> 27)) ^ acc`.
/// - `Multiply`    (algorithm spec §7.6(B)): `key * 0xA5555529 + acc` (`0xA5555529` = `-1521134295i32`).
/// - `*Swapped`: same shapes, but `acc` is the one mixed and `key` is the one combined in.
pub fn combine(form: CombineForm, acc: i32, key: i32) -> i32 {
    const MULTIPLIER: i32 = -1521134295; // 0xA5555529 as i32
    match form {
        CombineForm::RotateXor => {
            ((key << 5).wrapping_add(key).wrapping_add(key >> 27)) ^ acc
        }
        CombineForm::Multiply => key.wrapping_mul(MULTIPLIER).wrapping_add(acc),
        CombineForm::RotateXorSwapped => {
            ((acc << 5).wrapping_add(acc).wrapping_add(acc >> 27)) ^ key
        }
        CombineForm::MultiplySwapped => acc.wrapping_mul(MULTIPLIER).wrapping_add(key),
    }
}

/// `SnapshotLineDefinition.GetHashCode` (algorithm spec §7.6) = `Combine(From.Line, Id.GetHashCode())`
/// — mixes `id_hash`, combines with `from_line`. `From.Line` is a `ushort`; its `.GetHashCode()`
/// in .NET is the value itself, so `from_line as i32` is exact.
pub fn line_get_hash_code(form: CombineForm, from_line: u16, id_hash: i32) -> i32 {
    combine(form, from_line as i32, id_hash)
}

/// `CpuProfileCallFrameDefinition.GetHashCode` (algorithm spec §7.6) =
/// `Combine(LineDefinition?.GetHashCode() ?? 0, ExecutionIndex)` — mixes `execution_index`,
/// combines with `line_hash`. Caller passes `line_hash = 0` when `LineDefinition` is `None`.
pub fn frame_get_hash_code(form: CombineForm, line_hash: i32, execution_index: i32) -> i32 {
    combine(form, line_hash, execution_index)
}

/// `CreateCallFrameIdentifier` (algorithm spec §7.6): fold the frame's own `GetHashCode` with every
/// ancestor's `GetHashCode`, in ancestor order (nearest first). `chain_hashes[0]` is the
/// frame's own hash; `chain_hashes[1..]` are its ancestors' hashes, nearest-first. Each fold
/// step is `h = Combine(h, parent_hash)` — mixes `parent_hash`, combines with the running `h`.
pub fn frame_identifier(form: CombineForm, chain_hashes: &[i32]) -> i32 {
    let mut h = chain_hashes[0];
    for &parent_hash in &chain_hashes[1..] {
        h = combine(form, h, parent_hash);
    }
    h
}
