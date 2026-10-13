use std::io;
use std::path::PathBuf;

pub struct ContentStore {
    root: PathBuf,
}

impl ContentStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn put(&self, bytes: &[u8], ext: &str) -> io::Result<String> {
        let hash = blake3::hash(bytes).to_hex().to_string();
        let path = self.path_for(&hash, ext)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(hash)
    }

    pub fn get(&self, hash: &str, ext: &str) -> io::Result<Vec<u8>> {
        let path = self.path_for(hash, ext)?;
        std::fs::read(&path)
    }

    fn path_for(&self, hash: &str, ext: &str) -> io::Result<PathBuf> {
        validate_hash(hash)?;
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "invalid blake3 hash");
        let shard_a = hash.get(..2).ok_or_else(invalid)?;
        let shard_b = hash.get(2..4).ok_or_else(invalid)?;
        let clean = sanitize_ext(ext);
        Ok(self
            .root
            .join(shard_a)
            .join(shard_b)
            .join(format!("{hash}.{clean}")))
    }
}

fn sanitize_ext(ext: &str) -> String {
    let cleaned: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if cleaned.is_empty() {
        String::from("bin")
    } else {
        cleaned
    }
}

fn validate_hash(hash: &str) -> io::Result<()> {
    let ok = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid blake3 hash",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_root(tag: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("asset-hub-{tag}-{pid}-{nanos}"))
    }

    #[test]
    fn round_trip_bin() {
        let root = unique_root("roundtrip");
        let store = ContentStore::new(root.clone());
        let bytes = b"hello game-studio";
        let hash = store.put(bytes, "bin").unwrap();
        assert_eq!(hash, blake3::hash(bytes).to_hex().to_string());
        let back = store.get(&hash, "bin").unwrap();
        assert_eq!(back, bytes);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_bad_hash_and_shards_path() {
        let root = unique_root("shard");
        let store = ContentStore::new(root.clone());
        let bytes = b"hello game-studio";
        let hash = store.put(bytes, "bin").unwrap();
        let sharded = root
            .join(&hash[..2])
            .join(&hash[2..4])
            .join(format!("{hash}.bin"));
        assert!(sharded.is_file());
        for bad in ["", "abc", "not-a-hash", &"z".repeat(64), &"0".repeat(63)] {
            let err = store.get(bad, "bin").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sanitizes_ext_and_defaults_to_bin() {
        let root = unique_root("ext");
        let store = ContentStore::new(root.clone());
        let bytes = b"hello game-studio";
        let hash = store.put(bytes, "").unwrap();
        let default_path = root
            .join(&hash[..2])
            .join(&hash[2..4])
            .join(format!("{hash}.bin"));
        assert!(default_path.is_file());
        assert_eq!(store.get(&hash, "").unwrap(), bytes);
        let hash2 = store.put(bytes, "../p!n@g").unwrap();
        assert_eq!(hash2, hash);
        assert_eq!(store.get(&hash, "png").unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&root);
    }
}
