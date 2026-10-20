//! Cheap file fingerprints: size plus a hash of the first and last megabyte.
//!
//! Used to spot duplicates and to recognise a movie that was moved or renamed.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const CHUNK: u64 = 1024 * 1024;

pub fn fingerprint(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());

    let head_len = size.min(CHUNK);
    let mut buf = vec![0u8; head_len as usize];
    file.read_exact(&mut buf)?;
    hasher.update(&buf);

    if size > CHUNK {
        // The tail chunk never overlaps the head chunk.
        let tail_start = size.saturating_sub(CHUNK).max(CHUNK);
        file.seek(SeekFrom::Start(tail_start))?;
        buf.clear();
        file.read_to_end(&mut buf)?;
        hasher.update(&buf);
    }
    Ok(format!("{size:x}-{}", &hasher.finalize().to_hex()[..32]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn identical_content_gives_identical_fingerprints() {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let a = write(dir.path(), "a.mkv", &data);
        let b = write(dir.path(), "b.mkv", &data);
        assert_eq!(fingerprint(&a).unwrap(), fingerprint(&b).unwrap());
    }

    #[test]
    fn a_change_in_the_tail_changes_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let mut data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let a = write(dir.path(), "a.mkv", &data);
        *data.last_mut().unwrap() ^= 0xff;
        let b = write(dir.path(), "b.mkv", &data);
        assert_ne!(fingerprint(&a).unwrap(), fingerprint(&b).unwrap());
    }

    #[test]
    fn small_files_work() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a.mp4", b"tiny");
        let b = write(dir.path(), "b.mp4", b"tinz");
        assert_ne!(fingerprint(&a).unwrap(), fingerprint(&b).unwrap());
        assert!(fingerprint(&a).unwrap().starts_with("4-"));
    }
}
