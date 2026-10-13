use std::io;
use std::path::PathBuf;

pub struct ContentStore {
    root: PathBuf,
}

impl ContentStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
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
