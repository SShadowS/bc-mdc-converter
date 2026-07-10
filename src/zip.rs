use crate::{Error, Result};
use std::io::Read;

pub struct ZipEntry {
    pub name: String,
    pub data: Vec<u8>,
    /// Entry's DOS mod time as `YYYY-MM-DDTHH:MM:SS` — the CAPTURING machine's local
    /// time, timezone unknown, 2-second resolution (spec §3.2 `approxWallClockStart`).
    /// `None` when the header carries no timestamp or the DOS zero placeholder.
    pub mtime_iso: Option<String>,
}

pub fn read_zip_entries(bytes: &[u8]) -> Result<Vec<ZipEntry>> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = ::zip::ZipArchive::new(reader).map_err(|e| Error::Zip(e.to_string()))?;
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| Error::Zip(e.to_string()))?;
        let name = file.name().to_string();
        let mtime_iso = file.last_modified().and_then(dos_datetime_to_iso);
        let mut data = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut data)?;
        out.push(ZipEntry { name, data, mtime_iso });
    }
    Ok(out)
}

/// `None` for the DOS zero placeholder (1980-01-01 00:00:00): writers that store no real
/// timestamp emit exactly that value, and reporting it as a wall clock would be a lie.
fn dos_datetime_to_iso(dt: ::zip::DateTime) -> Option<String> {
    let (y, mo, d, h, mi, s) =
        (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second());
    if (y, mo, d, h, mi, s) == (1980, 1, 1, 0, 0, 0) {
        return None;
    }
    Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}"))
}

#[cfg(test)]
mod tests {
    use super::dos_datetime_to_iso;

    #[test]
    fn formats_real_timestamp() {
        let dt = ::zip::DateTime::from_date_and_time(2024, 1, 17, 9, 31, 22).unwrap();
        assert_eq!(dos_datetime_to_iso(dt).as_deref(), Some("2024-01-17T09:31:22"));
    }

    #[test]
    fn dos_zero_placeholder_is_none() {
        // 1980-01-01 00:00:00 is the DOS packed-time zero — an absent timestamp, not a real one.
        let dt = ::zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(dos_datetime_to_iso(dt), None);
    }
}
