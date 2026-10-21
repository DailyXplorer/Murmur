//! Whether a file on disk is the one the manifest names.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

use super::manifest::ModelFile;

pub fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        match file.read(&mut buffer)? {
            0 => break,
            read => hasher.update(&buffer[..read]),
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// `false` for a missing file, or one of the wrong size or content.
pub fn matches(path: &Path, expected: &ModelFile) -> std::io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() == expected.size => Ok(sha256_of(path)? == expected.sha256),
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_matches_only_with_its_size_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        let expected = ModelFile {
            path: "file",
            size: 3,
            // SHA-256 of "abc".
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        assert!(!matches(&path, &expected).unwrap());
        std::fs::write(&path, b"abc").unwrap();
        assert!(matches(&path, &expected).unwrap());
        std::fs::write(&path, b"abd").unwrap();
        assert!(!matches(&path, &expected).unwrap());
        std::fs::write(&path, b"abcd").unwrap();
        assert!(!matches(&path, &expected).unwrap());
    }
}
