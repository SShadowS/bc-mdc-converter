// Minimal FlatBuffers reader over a trusted buffer (the profiler output).
// All little-endian. No file_identifier, not size-prefixed: root uoffset at byte 0.
#[derive(Clone, Copy)]
pub struct Table<'a> {
    buf: &'a [u8],
    pos: usize,
}

#[inline]
fn u16le(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}
#[inline]
fn i32le(b: &[u8], p: usize) -> i32 {
    i32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
}
#[inline]
fn i64le(b: &[u8], p: usize) -> i64 {
    i64::from_le_bytes([
        b[p],
        b[p + 1],
        b[p + 2],
        b[p + 3],
        b[p + 4],
        b[p + 5],
        b[p + 6],
        b[p + 7],
    ])
}

impl<'a> Table<'a> {
    pub fn root(buf: &'a [u8]) -> Table<'a> {
        let pos = i32le(buf, 0) as usize; // root uoffset
        Table { buf, pos }
    }

    /// Absolute buffer offset of field `id`'s data, or None if the slot is 0 (absent).
    pub fn field_offset(&self, id: u16) -> Option<usize> {
        let vtable = (self.pos as i64 - i32le(self.buf, self.pos) as i64) as usize;
        let vt_size = u16le(self.buf, vtable) as usize;
        let slot = 4 + 2 * id as usize;
        if slot >= vt_size {
            return None;
        }
        let field_rel = u16le(self.buf, vtable + slot);
        if field_rel == 0 {
            return None;
        }
        Some(self.pos + field_rel as usize)
    }

    pub fn scalar_i8(&self, id: u16, default: i8) -> i8 {
        self.field_offset(id).map(|o| self.buf[o] as i8).unwrap_or(default)
    }
    pub fn scalar_i32(&self, id: u16, default: i32) -> i32 {
        self.field_offset(id).map(|o| i32le(self.buf, o)).unwrap_or(default)
    }
    pub fn scalar_i64(&self, id: u16, default: i64) -> i64 {
        self.field_offset(id).map(|o| i64le(self.buf, o)).unwrap_or(default)
    }
    pub fn string(&self, id: u16) -> Option<&'a str> {
        let fo = self.field_offset(id)?;
        let p = fo + i32le(self.buf, fo) as usize; // follow uoffset
        let len = i32le(self.buf, p) as usize;
        std::str::from_utf8(&self.buf[p + 4..p + 4 + len]).ok()
    }
    pub fn table(&self, id: u16) -> Option<Table<'a>> {
        let fo = self.field_offset(id)?;
        let p = fo + i32le(self.buf, fo) as usize; // __indirect
        Some(Table { buf: self.buf, pos: p })
    }
    pub fn vector_len(&self, id: u16) -> usize {
        match self.field_offset(id) {
            Some(fo) => {
                let v = fo + i32le(self.buf, fo) as usize;
                i32le(self.buf, v) as usize
            }
            None => 0,
        }
    }
    pub fn vector_elem_table(&self, id: u16, i: usize) -> Option<Table<'a>> {
        let fo = self.field_offset(id)?;
        let v = fo + i32le(self.buf, fo) as usize; // vector start (points at len)
        let base = v + 4 + i * 4; // element uoffset slot
        let p = base + i32le(self.buf, base) as usize; // element table pos
        Some(Table { buf: self.buf, pos: p })
    }
    /// Read an inline struct's scalar at (field data offset) + byte_off. Struct fields
    /// are stored in-place at the vtable slot, NOT via an indirection.
    pub fn struct_i64(&self, id: u16, byte_off: usize) -> Option<i64> {
        self.field_offset(id).map(|fo| i64le(self.buf, fo + byte_off))
    }
    pub fn struct_u16(&self, id: u16, byte_off: usize) -> Option<u16> {
        self.field_offset(id).map(|fo| u16le(self.buf, fo + byte_off))
    }
}
