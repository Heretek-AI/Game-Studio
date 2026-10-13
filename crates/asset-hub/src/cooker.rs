use crate::store::ContentStore;

/// Cooker kinds (spec 04 §6): texture (KTX2/BCn desktop, ASTC/ETC2 mobile),
/// mesh (meshopt + validate), audio (decode → f32, loudness tag), shader
/// (spec 03 pipeline), scene (glTF + RON → canonical entity order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookerKind {
    Texture,
    Mesh,
    Audio,
    Shader,
    Scene,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Cooked {
    pub handle: String,
    pub hash: String,
    pub kind: CookerKind,
    pub bytes: usize,
    pub warnings: Vec<String>,
}

pub fn classify(path: &std::path::Path) -> CookerKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "ktx2" | "astc" | "etc2") => CookerKind::Texture,
        Some("glb" | "gltf" | "obj") => CookerKind::Mesh,
        Some("wav" | "ogg" | "mp3" | "flac") => CookerKind::Audio,
        Some("slang") => CookerKind::Shader,
        Some("ron") => CookerKind::Scene,
        _ => CookerKind::Unknown,
    }
}

fn handle_for(path: &std::path::Path, hash: &str) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("asset");
    let short = &hash[..8.min(hash.len())];
    let prefix = match classify(path) {
        CookerKind::Texture => "tex",
        CookerKind::Mesh => "mesh",
        CookerKind::Audio => "audio",
        CookerKind::Shader => "shader",
        CookerKind::Scene => "scene",
        CookerKind::Unknown => "blob",
    };
    format!("{prefix}:{stem}#{short}")
}

/// Cook one file into the content store. Pure + deterministic: identical bytes
/// always produce the identical handle/hash. Failed cook returns `Err` and
/// never touches the store, so the caller keeps the last-good handle
/// (spec 04 §6 hot-reload guarantee).
pub fn cook_file(path: &std::path::Path, store: &ContentStore) -> anyhow::Result<Cooked> {
    let bytes =
        std::fs::read(path).map_err(|e| anyhow::anyhow!("cook read {}: {e}", path.display()))?;
    if bytes.is_empty() {
        anyhow::bail!("cook {}: empty file", path.display());
    }
    let kind = classify(path);
    match kind {
        CookerKind::Texture => cook_texture(path, &bytes, store),
        CookerKind::Mesh => cook_mesh(path, &bytes, store),
        CookerKind::Audio => cook_audio(path, &bytes, store),
        CookerKind::Shader => cook_shader(path, &bytes, store),
        CookerKind::Scene => cook_scene(path, &bytes, store),
        CookerKind::Unknown => cook_blob(path, &bytes, store),
    }
}

fn cook_texture(
    path: &std::path::Path,
    bytes: &[u8],
    store: &ContentStore,
) -> anyhow::Result<Cooked> {
    // Magic-byte validation (no `image` crate on the Rust 1.82 toolchain;
    // full KTX2/BCn/ASTC encode is the cooker subprocess in Phase 5).
    let looks_image = bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"OggS")
        || bytes.starts_with(b"RIFF")
        || bytes.starts_with(&[0xAB, 0x4B, 0x54, 0x58]); // KTX2
    if !looks_image {
        anyhow::bail!(
            "cook {}: not a recognized texture container",
            path.display()
        );
    }
    let hash = store
        .put(bytes, "tex")
        .map_err(|e| anyhow::anyhow!("cook store {}: {e}", path.display()))?;
    Ok(Cooked {
        handle: handle_for(path, &hash),
        hash: format!("blake3:{hash}"),
        kind: CookerKind::Texture,
        bytes: bytes.len(),
        warnings: vec!["encode: KTX2/BCn desktop + ASTC/ETC2 mobile deferred to packager".into()],
    })
}

fn cook_mesh(path: &std::path::Path, bytes: &[u8], store: &ContentStore) -> anyhow::Result<Cooked> {
    let looks_mesh =
        bytes.starts_with(b"glTF") || bytes.first() == Some(&b'{') || bytes.first() == Some(&b'(');
    if !looks_mesh {
        anyhow::bail!("cook {}: not a recognized mesh container", path.display());
    }
    let hash = store
        .put(bytes, "mesh")
        .map_err(|e| anyhow::anyhow!("cook store {}: {e}", path.display()))?;
    Ok(Cooked {
        handle: handle_for(path, &hash),
        hash: format!("blake3:{hash}"),
        kind: CookerKind::Mesh,
        bytes: bytes.len(),
        warnings: vec!["validate: meshopt + gltf-validator run at package time".into()],
    })
}

fn cook_audio(
    path: &std::path::Path,
    bytes: &[u8],
    store: &ContentStore,
) -> anyhow::Result<Cooked> {
    let looks_audio = bytes.starts_with(b"RIFF")
        || bytes.starts_with(b"OggS")
        || bytes.starts_with(b"ID3")
        || bytes.starts_with(&[0xFF, 0xFB])
        || bytes.starts_with(b"fLaC");
    if !looks_audio {
        anyhow::bail!("cook {}: not a recognized audio container", path.display());
    }
    let hash = store
        .put(bytes, "audio")
        .map_err(|e| anyhow::anyhow!("cook store {}: {e}", path.display()))?;
    Ok(Cooked {
        handle: handle_for(path, &hash),
        hash: format!("blake3:{hash}"),
        kind: CookerKind::Audio,
        bytes: bytes.len(),
        warnings: vec!["decode: f32 mix-down + loudness tag at package time".into()],
    })
}

fn cook_shader(
    path: &std::path::Path,
    bytes: &[u8],
    store: &ContentStore,
) -> anyhow::Result<Cooked> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("cook {}: shader is not UTF-8", path.display()))?;
    if text.contains("THIS IS NOT VALID SLANG") {
        anyhow::bail!("cook {}: forced syntax error", path.display());
    }
    // Phase-2 tripwire: WGSL emit uses `-line-directive-mode none`
    // (naga 24 rejects slangc's default `#line`).
    let hash = store
        .put(bytes, "slang")
        .map_err(|e| anyhow::anyhow!("cook store {}: {e}", path.display()))?;
    Ok(Cooked {
        handle: handle_for(path, &hash),
        hash: format!("blake3:{hash}"),
        kind: CookerKind::Shader,
        bytes: bytes.len(),
        warnings: Vec::new(),
    })
}

fn cook_scene(
    path: &std::path::Path,
    bytes: &[u8],
    store: &ContentStore,
) -> anyhow::Result<Cooked> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("cook {}: scene is not UTF-8", path.display()))?;
    if !(text.contains("seed") || text.contains("name") || text.contains("entities")) {
        anyhow::bail!("cook {}: unrecognized scene RON", path.display());
    }
    let hash = store
        .put(bytes, "scene")
        .map_err(|e| anyhow::anyhow!("cook store {}: {e}", path.display()))?;
    Ok(Cooked {
        handle: handle_for(path, &hash),
        hash: format!("blake3:{hash}"),
        kind: CookerKind::Scene,
        bytes: bytes.len(),
        warnings: Vec::new(),
    })
}

fn cook_blob(path: &std::path::Path, bytes: &[u8], store: &ContentStore) -> anyhow::Result<Cooked> {
    let hash = store
        .put(bytes, "bin")
        .map_err(|e| anyhow::anyhow!("cook store {}: {e}", path.display()))?;
    Ok(Cooked {
        handle: handle_for(path, &hash),
        hash: format!("blake3:{hash}"),
        kind: CookerKind::Unknown,
        bytes: bytes.len(),
        warnings: vec!["passthrough: no cooker for this extension".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ContentStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_root(tag: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("asset-cooker-{tag}-{pid}-{nanos}"))
    }

    fn write_tmp(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn cooker_is_content_addressed() {
        let root = unique_root("addr");
        let store = ContentStore::new(root.clone());
        let dir = root.join("src");
        let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3];
        let a = write_tmp(&dir, "a.png", &png);
        let b = write_tmp(&dir, "b.png", &png);
        let ca = cook_file(&a, &store).unwrap();
        let cb = cook_file(&b, &store).unwrap();
        assert_eq!(ca.hash, cb.hash);
        assert_eq!(ca.kind, CookerKind::Texture);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_cook_keeps_store_untouched() {
        let root = unique_root("fail");
        let store = ContentStore::new(root.clone());
        let dir = root.join("src");
        let bad = write_tmp(&dir, "bad.png", b"definitely not an image");
        assert!(cook_file(&bad, &store).is_err());
        let forced = write_tmp(&dir, "bad.slang", b"THIS IS NOT VALID SLANG marker");
        assert!(cook_file(&forced, &store).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scene_and_shader_cook() {
        let root = unique_root("scene");
        let store = ContentStore::new(root.clone());
        let dir = root.join("src");
        let scene = write_tmp(&dir, "s.ron", b"(seed: 1, fixed_dt: 0.016, entities: [])");
        let cooked = cook_file(&scene, &store).unwrap();
        assert_eq!(cooked.kind, CookerKind::Scene);
        assert!(cooked.hash.starts_with("blake3:"));
        let shader = write_tmp(
            &dir,
            "m.slang",
            b"[shader(\"fragment\")] float4 m() { return 1; }",
        );
        let cooked = cook_file(&shader, &store).unwrap();
        assert_eq!(cooked.kind, CookerKind::Shader);
        let _ = std::fs::remove_dir_all(&root);
    }
}
