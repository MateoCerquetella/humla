//! When an imported recording was made (#222), so the note lands at that day
//! instead of the day it was imported.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// 2001-01-01. Anything earlier is an unset clock, not a meeting.
const EARLIEST_PLAUSIBLE_MS: i64 = 978_307_200_000;
const DAY_MS: i64 = 86_400_000;
const MAC_EPOCH_OFFSET_SECS: u64 = 2_082_844_800;

/// The MP4 movie header's creation time (m4a, mp4, mov — what Voice Memos and
/// most phones write), else the earliest of the file's own timestamps, since a
/// copy resets one or the other. `None` when nothing plausible is there.
pub fn recording_date(path: &Path, now_ms: i64) -> Option<i64> {
    let plausible = |ms: i64| (EARLIEST_PLAUSIBLE_MS..=now_ms + DAY_MS).contains(&ms);
    if let Some(ms) = File::open(path).ok().and_then(|mut f| mvhd_creation_ms(&mut f)) {
        if plausible(ms) {
            return Some(ms);
        }
    }
    let meta = std::fs::metadata(path).ok()?;
    [meta.created().ok(), meta.modified().ok()]
        .into_iter()
        .flatten()
        .filter_map(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .filter(|&ms| plausible(ms))
        .min()
}

fn mvhd_creation_ms<R: Read + Seek>(r: &mut R) -> Option<i64> {
    let end = r.seek(SeekFrom::End(0)).ok()?;
    let (moov_start, moov_end) = find_box(r, 0, end, b"moov")?;
    let (mvhd_start, _) = find_box(r, moov_start, moov_end, b"mvhd")?;
    r.seek(SeekFrom::Start(mvhd_start)).ok()?;
    let mut version = [0u8; 4];
    r.read_exact(&mut version).ok()?;
    let secs = if version[0] == 1 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).ok()?;
        u64::from_be_bytes(b)
    } else {
        let mut b = [0u8; 4];
        r.read_exact(&mut b).ok()?;
        u32::from_be_bytes(b) as u64
    };
    let unix = secs.checked_sub(MAC_EPOCH_OFFSET_SECS)?;
    i64::try_from(unix).ok()?.checked_mul(1000)
}

/// The payload range of the first `kind` box among the boxes in `start..end`.
fn find_box<R: Read + Seek>(r: &mut R, start: u64, end: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = start;
    while pos + 8 <= end {
        r.seek(SeekFrom::Start(pos)).ok()?;
        let mut head = [0u8; 8];
        r.read_exact(&mut head).ok()?;
        let mut size = u32::from_be_bytes(head[..4].try_into().ok()?) as u64;
        let mut header = 8;
        if size == 1 {
            let mut large = [0u8; 8];
            r.read_exact(&mut large).ok()?;
            size = u64::from_be_bytes(large);
            header = 16;
        } else if size == 0 {
            size = end - pos;
        }
        if size < header || pos + size > end {
            return None;
        }
        if &head[4..] == kind {
            return Some((pos + header, pos + size));
        }
        pos += size;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::recording_date;
    use std::fs::File;
    use std::io::Write;
    use std::time::{Duration, UNIX_EPOCH};

    const NOW: i64 = 1_790_000_000_000; // 2026-09
    const MAC_EPOCH_OFFSET: u64 = 2_082_844_800; // 1904-01-01 → 1970-01-01, in seconds

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(payload);
        b
    }

    fn mvhd_v0(unix_secs: u64) -> Vec<u8> {
        let mut p = vec![0, 0, 0, 0]; // version 0, flags
        p.extend_from_slice(&((unix_secs + MAC_EPOCH_OFFSET) as u32).to_be_bytes());
        p.extend_from_slice(&[0; 92]);
        mp4_box(b"mvhd", &p)
    }

    fn mvhd_v1(unix_secs: u64) -> Vec<u8> {
        let mut p = vec![1, 0, 0, 0];
        p.extend_from_slice(&(unix_secs + MAC_EPOCH_OFFSET).to_be_bytes());
        p.extend_from_slice(&[0; 104]);
        mp4_box(b"mvhd", &p)
    }

    fn write_file(dir: &tempfile::TempDir, name: &str, bytes: &[u8], mtime_secs: Option<u64>) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        if let Some(s) = mtime_secs {
            f.set_modified(UNIX_EPOCH + Duration::from_secs(s)).unwrap();
        }
        path
    }

    #[test]
    fn reads_the_movie_header_of_an_m4a() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = mp4_box(b"ftyp", b"M4A \0\0\0\0");
        bytes.extend(mp4_box(b"moov", &mvhd_v0(1_600_000_000)));
        let path = write_file(&dir, "memo.m4a", &bytes, None);
        assert_eq!(recording_date(&path, NOW), Some(1_600_000_000_000));
    }

    /// Not every file is fast-started: the movie box can follow the media data.
    #[test]
    fn finds_a_version_1_header_after_the_media_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = mp4_box(b"ftyp", b"isom\0\0\0\0");
        bytes.extend(mp4_box(b"mdat", &[7; 4096]));
        let mut moov = mp4_box(b"udta", b"xxxx");
        moov.extend(mvhd_v1(1_650_000_000));
        bytes.extend(mp4_box(b"moov", &moov));
        let path = write_file(&dir, "call.mp4", &bytes, None);
        assert_eq!(recording_date(&path, NOW), Some(1_650_000_000_000));
    }

    /// An encoder that leaves the header at zero says nothing; the file's own
    /// timestamps are the next best answer, and the earliest of them, since a
    /// copy resets one or the other.
    #[test]
    fn falls_back_to_the_earliest_file_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = mp4_box(b"ftyp", b"M4A \0\0\0\0");
        bytes.extend(mp4_box(b"moov", &mvhd_v0(0)));
        let path = write_file(&dir, "zeroed.m4a", &bytes, Some(1_560_000_000));
        assert_eq!(recording_date(&path, NOW), Some(1_560_000_000_000));

        let path = write_file(&dir, "talk.mp3", b"ID3\x03\0\0\0\0\0\0", Some(1_570_000_000));
        assert_eq!(recording_date(&path, NOW), Some(1_570_000_000_000));
    }

    #[test]
    fn ignores_dates_from_the_future() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = mp4_box(b"ftyp", b"M4A \0\0\0\0");
        bytes.extend(mp4_box(b"moov", &mvhd_v0(1_900_000_000)));
        let path = write_file(&dir, "future.m4a", &bytes, Some(1_580_000_000));
        assert_eq!(recording_date(&path, NOW), Some(1_580_000_000_000));
    }
}
