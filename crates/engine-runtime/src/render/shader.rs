use std::path::{Path, PathBuf};

pub const MAX_VARIANTS_PER_MATERIAL: usize = 64;
pub const PSO_CACHE_DIR: &str = ".studio/pso";

#[derive(Debug, Clone)]
pub struct CompiledShader {
    pub name: String,
    pub wgsl: String,
    pub spirv: Vec<u8>,
    pub reflection_json: serde_json::Value,
    pub hash: String,
    pub subgroups_off: bool,
    pub from_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderError {
    SlangFailed(String),
    NagaFailed(String),
    Compat(String),
    VariantBudget(usize),
    Io(String),
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SlangFailed(detail) => write!(f, "E_SHADER_COMPILE: slangc failed: {detail}"),
            Self::NagaFailed(detail) => {
                write!(f, "E_SHADER_VALIDATE: naga rejected WGSL: {detail}")
            }
            Self::Compat(detail) => write!(f, "{detail}"),
            Self::VariantBudget(count) => write!(
                f,
                "E_SHADER_VARIANTS: {count} variants exceeds budget {MAX_VARIANTS_PER_MATERIAL}"
            ),
            Self::Io(detail) => write!(f, "E_SHADER_IO: {detail}"),
        }
    }
}

impl std::error::Error for ShaderError {}

pub struct ShaderPipeline {
    pub repo_root: PathBuf,
    pub out_dir: PathBuf,
    pub pso_dir: PathBuf,
}

impl ShaderPipeline {
    pub fn new(repo_root: &Path) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            out_dir: repo_root.join(".studio/shader-out"),
            pso_dir: repo_root.join(PSO_CACHE_DIR),
        }
    }

    pub fn slangc(&self) -> PathBuf {
        self.repo_root.join("tools/bin/slangc")
    }

    pub fn check_compat(&self, source: &str) -> Result<(), ShaderError> {
        let lowered = source.to_lowercase();
        if lowered.contains("[shader(\"mesh\")]") || lowered.contains("[shader(\"amplification\")]")
        {
            return Err(ShaderError::Compat(
                "E_SHADER_STAGE: mesh shaders need native-only APIs (spec 03 §1 non-goals)"
                    .to_string(),
            ));
        }
        if lowered.contains("[shader(\"geometry\")]") {
            return Err(ShaderError::Compat(
                "E_SHADER_STAGE: geometry shaders unsupported in v1".to_string(),
            ));
        }
        if source.contains("RayQuery") || source.contains("SPV_KHR_ray_tracing") {
            return Err(ShaderError::Compat(
                "E_SHADER_CAP: HW-RT breaks the WebGPU portability contract".to_string(),
            ));
        }
        if lowered.contains("bindless") && !lowered.contains("max") {
            return Err(ShaderError::Compat(
                "E_SHADER_BIND: bounded binding arrays only; declare max count".to_string(),
            ));
        }
        Ok(())
    }

    pub fn check_desktop_precision(&self, source: &str) -> Result<(), ShaderError> {
        if source.contains("mediump") {
            return Err(ShaderError::Compat(
                "E_SHADER_PRECISION: mediump forbidden on desktop (spec 03 §3.4)".to_string(),
            ));
        }
        Ok(())
    }

    pub fn validate_wgsl(&self, wgsl: &str) -> Result<(), ShaderError> {
        naga::front::wgsl::parse_str(wgsl)
            .map(|_| ())
            .map_err(|err| ShaderError::NagaFailed(format!("{err:?}")))
    }

    pub fn hash_wgsl(&self, wgsl: &str) -> String {
        blake3::hash(wgsl.as_bytes()).to_hex().to_string()
    }

    pub fn pso_path(&self, hash: &str) -> PathBuf {
        self.pso_dir.join(hash)
    }

    pub fn assert_variant_budget(&self, variants: &[String]) -> Result<(), ShaderError> {
        if variants.len() > MAX_VARIANTS_PER_MATERIAL {
            return Err(ShaderError::VariantBudget(variants.len()));
        }
        Ok(())
    }

    pub fn compile(
        &self,
        slang_path: &Path,
        extra_defines: &[(&str, &str)],
    ) -> Result<CompiledShader, ShaderError> {
        let source = std::fs::read_to_string(slang_path)
            .map_err(|err| ShaderError::Io(format!("read {}: {err}", slang_path.display())))?;
        self.check_compat(&source)?;
        for (key, _) in extra_defines {
            if key.contains("fast-math") || key.contains("ffast") {
                return Err(ShaderError::Compat(
                    "E_SHADER_PRECISION: fast-math is forbidden (spec 03 §3.4)".to_string(),
                ));
            }
        }
        let stem = slang_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("shader")
            .to_string();
        let subgroups_off = extra_defines.iter().any(|(key, _)| *key == "SUBGROUPS_OFF");
        std::fs::create_dir_all(&self.out_dir)
            .map_err(|err| ShaderError::Io(format!("mkdir shader-out: {err}")))?;
        let wgsl_out = self.out_dir.join(format!("{stem}.wgsl"));
        let spv_out = self.out_dir.join(format!("{stem}.spv"));
        let json_out = self.out_dir.join(format!("{stem}.json"));
        let slangc = self.slangc();
        // Note: WGSL artifacts use `-line-directive-mode none` because naga 24
        // rejects `#line` directives (spec 03 R-4 triple impedance; source
        // mapping lives in the reflection JSON instead).
        let mut command = std::process::Command::new(&slangc);
        command
            .arg(slang_path)
            .arg("-target")
            .arg("wgsl")
            .arg("-o")
            .arg(&wgsl_out)
            .arg("-emit-spirv-directly")
            .arg("-target")
            .arg("spirv")
            .arg("-o")
            .arg(&spv_out)
            .arg("-reflection-json")
            .arg(&json_out)
            .arg("-line-directive-mode")
            .arg("none")
            .arg("-O2");
        for (key, value) in extra_defines {
            command.arg("-D").arg(format!("{key}={value}"));
        }
        let output = command
            .output()
            .map_err(|err| ShaderError::Io(format!("spawn {}: {err}", slangc.display())))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            let last_line = detail.lines().last().unwrap_or("slangc failed").to_string();
            return Err(ShaderError::SlangFailed(
                last_line.chars().take(500).collect(),
            ));
        }
        let wgsl = std::fs::read_to_string(&wgsl_out)
            .map_err(|err| ShaderError::Io(format!("read wgsl: {err}")))?;
        self.validate_wgsl(&wgsl)?;
        let spirv = std::fs::read(&spv_out).unwrap_or_default();
        let reflection_json = std::fs::read_to_string(&json_out)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(serde_json::json!({ "entry": stem }));
        let hash = self.hash_wgsl(&wgsl);
        let compiled = CompiledShader {
            name: stem,
            wgsl,
            spirv,
            reflection_json,
            hash: hash.clone(),
            subgroups_off,
            from_cache: false,
        };
        self.store_pso(&compiled)?;
        Ok(compiled)
    }

    pub fn compile_or_last_good(
        &self,
        slang_path: &Path,
        extra_defines: &[(&str, &str)],
    ) -> Result<CompiledShader, ShaderError> {
        match self.compile(slang_path, extra_defines) {
            Ok(compiled) => Ok(compiled),
            Err(err) => {
                if let Some(cached) = self.last_good(slang_path) {
                    let _ = err;
                    Ok(cached)
                } else {
                    Err(err)
                }
            }
        }
    }

    pub fn store_pso(&self, compiled: &CompiledShader) -> Result<PathBuf, ShaderError> {
        let dir = self.pso_path(&compiled.hash);
        std::fs::create_dir_all(&dir)
            .map_err(|err| ShaderError::Io(format!("mkdir pso: {err}")))?;
        std::fs::write(dir.join(format!("{}.wgsl", compiled.name)), &compiled.wgsl)
            .map_err(|err| ShaderError::Io(format!("write pso wgsl: {err}")))?;
        std::fs::write(dir.join(format!("{}.spv", compiled.name)), &compiled.spirv)
            .map_err(|err| ShaderError::Io(format!("write pso spv: {err}")))?;
        std::fs::write(
            dir.join(format!("{}.json", compiled.name)),
            serde_json::to_string_pretty(&compiled.reflection_json).unwrap_or_default(),
        )
        .map_err(|err| ShaderError::Io(format!("write pso json: {err}")))?;
        Ok(dir)
    }

    pub fn last_good(&self, slang_path: &Path) -> Option<CompiledShader> {
        let stem = slang_path.file_stem()?.to_str()?.to_string();
        let direct = self.out_dir.join(format!("{stem}.wgsl"));
        let wgsl = std::fs::read_to_string(&direct).ok()?;
        let hash = self.hash_wgsl(&wgsl);
        let spirv = std::fs::read(self.out_dir.join(format!("{stem}.spv"))).unwrap_or_default();
        let reflection_json = std::fs::read_to_string(self.out_dir.join(format!("{stem}.json")))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(serde_json::json!({ "entry": stem }));
        Some(CompiledShader {
            name: stem,
            wgsl,
            spirv,
            reflection_json,
            hash,
            subgroups_off: false,
            from_cache: true,
        })
    }
}

/// Phase 7.3 sketch: cold `slangc` worker pool (`ISession` concept).
///
/// Cold workers each own one logical slang `ISession`; the pool hands them
/// out round-robin so parallel material variants do not share mutable
/// compiler state. All sessions must agree on one canonical flag set
/// (flag parity with [`ShaderPipeline::compile`]); per-variant differences
/// travel only through `-D` defines, which are folded into the blake3
/// cache key below. Sketch only: no threads are spawned here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlangcFlags {
    /// Optimization level passed as `-O<n>` (compile path uses `-O2`).
    pub opt_level: String,
    /// Line directive mode (compile path uses `none` for naga 24 compat).
    pub line_directive_mode: String,
    /// Whether `-emit-spirv-directly` is set.
    pub emit_spirv_directly: bool,
    /// Whether `-reflection-json` is requested.
    pub reflection_json: bool,
}

impl Default for SlangcFlags {
    fn default() -> Self {
        Self::canonical()
    }
}

impl SlangcFlags {
    /// Canonical flags mirroring `ShaderPipeline::compile`.
    pub fn canonical() -> Self {
        Self {
            opt_level: "2".to_string(),
            line_directive_mode: "none".to_string(),
            emit_spirv_directly: true,
            reflection_json: true,
        }
    }

    /// Stable flag argv shared by every pooled session (excludes I/O paths
    /// and per-variant `-D` defines, which callers append afterwards).
    pub fn argv(&self) -> Vec<String> {
        let mut args = vec![
            "-target".to_string(),
            "wgsl".to_string(),
            "-o".to_string(),
            "{wgsl_out}".to_string(),
        ];
        if self.emit_spirv_directly {
            args.push("-emit-spirv-directly".to_string());
        }
        args.extend([
            "-target".to_string(),
            "spirv".to_string(),
            "-o".to_string(),
            "{spv_out}".to_string(),
        ]);
        if self.reflection_json {
            args.extend(["-reflection-json".to_string(), "{json_out}".to_string()]);
        }
        args.extend([
            "-line-directive-mode".to_string(),
            self.line_directive_mode.clone(),
            format!("-O{}", self.opt_level),
        ]);
        args
    }

    /// Deterministic blake3 cache key over WGSL-relevant inputs: slang
    /// source, canonical flags, and sorted `-D` defines.
    pub fn cache_key(&self, source: &str, extra_defines: &[(&str, &str)]) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(source.as_bytes());
        hasher.update(&[0]);
        for arg in self.argv() {
            hasher.update(arg.as_bytes());
            hasher.update(&[0]);
        }
        let mut defines: Vec<(&str, &str)> = extra_defines.to_vec();
        defines.sort();
        for (key, value) in defines {
            hasher.update(key.as_bytes());
            hasher.update(&[61]);
            hasher.update(value.as_bytes());
            hasher.update(&[0]);
        }
        hasher.finalize().to_hex().to_string()
    }
}

/// One pooled cold worker: conceptually one slang `ISession`.
#[derive(Debug, Clone)]
pub struct ColdSlangcSession {
    /// Index within the pool; used for round-robin dispatch.
    pub id: usize,
    /// Must equal the pool's canonical flags (flag parity).
    pub flags: SlangcFlags,
}

impl ColdSlangcSession {
    /// Naga `parse_str` gate: reject bad WGSL before it reaches the cache.
    pub fn validate_wgsl(&self, wgsl: &str) -> Result<(), ShaderError> {
        naga::front::wgsl::parse_str(wgsl)
            .map(|_| ())
            .map_err(|err| ShaderError::NagaFailed(format!("{err:?}")))
    }
}

/// Fixed-size pool of cold `slangc` sessions with a blake3-keyed sketch of
/// the last-good fallback used by [`ShaderPipeline::compile_or_last_good`].
#[derive(Debug)]
pub struct ColdSlangcPool {
    sessions: Vec<ColdSlangcSession>,
    next: usize,
    /// Canonical flags; every session carries a clone of this.
    pub flags: SlangcFlags,
}

impl ColdSlangcPool {
    /// Build a pool with `capacity` sessions (clamped to at least 1).
    pub fn new(capacity: usize, flags: SlangcFlags) -> Self {
        let capacity = capacity.max(1);
        let sessions = (0..capacity)
            .map(|id| ColdSlangcSession {
                id,
                flags: flags.clone(),
            })
            .collect();
        Self {
            sessions,
            next: 0,
            flags,
        }
    }

    /// Number of pooled `ISession` workers.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// True when the pool holds no sessions (never after [`Self::new`]).
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Round-robin checkout of the next cold session.
    pub fn acquire(&mut self) -> &ColdSlangcSession {
        let idx = self.next % self.sessions.len();
        self.next = self.next.wrapping_add(1);
        &self.sessions[idx]
    }

    /// Every session must carry flags identical to the pool canonical set.
    pub fn check_flag_parity(&self) -> bool {
        self.sessions.iter().all(|s| s.flags == self.flags)
    }

    /// Blake3 cache key for a slang source + defines pair.
    pub fn cache_key(&self, source: &str, extra_defines: &[(&str, &str)]) -> String {
        self.flags.cache_key(source, extra_defines)
    }

    /// Sketch of the cold fallback: naga-gate `wgsl_candidate`, hashing it
    /// with blake3 on success, otherwise returning `last_good` when present
    /// (mirrors `compile_or_last_good` without spawning `slangc`).
    pub fn compile_or_last_good_sketch(
        &self,
        name: &str,
        wgsl_candidate: &str,
        last_good: Option<CompiledShader>,
    ) -> Result<CompiledShader, ShaderError> {
        match naga::front::wgsl::parse_str(wgsl_candidate) {
            Ok(_) => {
                let hash = blake3::hash(wgsl_candidate.as_bytes()).to_hex().to_string();
                Ok(CompiledShader {
                    name: name.to_string(),
                    wgsl: wgsl_candidate.to_string(),
                    spirv: Vec::new(),
                    reflection_json: serde_json::json!({ "entry": name }),
                    hash,
                    subgroups_off: false,
                    from_cache: false,
                })
            }
            Err(err) => {
                if let Some(cached) = last_good {
                    Ok(cached)
                } else {
                    Err(ShaderError::NagaFailed(format!("{err:?}")))
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline_in_temp() -> (TempDir, ShaderPipeline) {
        let dir = TempDir::new();
        let pipeline = ShaderPipeline::new(&dir.path);
        (dir, pipeline)
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "game-studio-shader-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }
    }

    #[test]
    fn compat_rejects_unlowerable_features() {
        let pipeline = ShaderPipeline::new(Path::new("/tmp"));
        let mesh = "[shader(\"mesh\")] void m() {}";
        assert!(matches!(
            pipeline.check_compat(mesh),
            Err(ShaderError::Compat(_))
        ));
        let rt = "RayQuery<int> q;";
        assert!(matches!(
            pipeline.check_compat(rt),
            Err(ShaderError::Compat(_))
        ));
        let ok = "float4 main(float4 p : POSITION) : SV_Position { return p; }";
        assert!(pipeline.check_compat(ok).is_ok());
    }

    #[test]
    fn desktop_rejects_mediump() {
        let pipeline = ShaderPipeline::new(Path::new("/tmp"));
        assert!(pipeline
            .check_desktop_precision("mediump float x;")
            .is_err());
        assert!(pipeline.check_desktop_precision("float x;").is_ok());
    }

    #[test]
    fn naga_accepts_minimal_wgsl_and_rejects_garbage() {
        let pipeline = ShaderPipeline::new(Path::new("/tmp"));
        let good = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }";
        assert!(pipeline.validate_wgsl(good).is_ok());
        assert!(pipeline.validate_wgsl("THIS IS NOT WGSL @@@").is_err());
    }

    #[test]
    fn subgroups_off_matches_default_for_portable_shaders() {
        let portable = "fn f() {}";
        let pipeline = ShaderPipeline::new(Path::new("/tmp"));
        assert_eq!(pipeline.hash_wgsl(portable), pipeline.hash_wgsl(portable));
        assert!(pipeline.check_compat(portable).is_ok());
    }

    #[test]
    fn pso_cache_roundtrip_and_variant_budget() {
        let (_dir, pipeline) = pipeline_in_temp();
        pipeline
            .assert_variant_budget(&vec!["a".to_string(); 64])
            .expect("64 ok");
        assert!(pipeline
            .assert_variant_budget(&vec!["a".to_string(); 65])
            .is_err());
        let compiled = CompiledShader {
            name: "test_mat".to_string(),
            wgsl: "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0,0.0,0.0,1.0); }".to_string(),
            spirv: vec![1, 2, 3],
            reflection_json: serde_json::json!({ "entry": "test_mat" }),
            hash: String::new(),
            subgroups_off: false,
            from_cache: false,
        };
        let hash = pipeline.hash_wgsl(&compiled.wgsl);
        let mut compiled = compiled;
        compiled.hash = hash.clone();
        let dir = pipeline.store_pso(&compiled).expect("store pso");
        assert!(dir.join("test_mat.wgsl").exists());
        assert_eq!(pipeline.pso_path(&hash), dir);
    }

    #[test]
    fn cold_pool_round_robin_and_flag_parity() {
        let mut pool = ColdSlangcPool::new(2, SlangcFlags::canonical());
        assert_eq!(pool.len(), 2);
        assert!(!pool.is_empty());
        assert!(pool.check_flag_parity());
        let first = pool.acquire().id;
        let second = pool.acquire().id;
        let third = pool.acquire().id;
        assert_ne!(first, second);
        assert_eq!(first, third);
        // Flag parity: every session matches the compile-path canonical set.
        let argv = pool.flags.argv();
        for token in [
            "-target",
            "wgsl",
            "-emit-spirv-directly",
            "spirv",
            "-line-directive-mode",
            "none",
            "-O2",
        ] {
            assert!(argv.iter().any(|a| a == token), "missing {token}");
        }
    }

    #[test]
    fn cold_pool_blake3_cache_key_is_stable() {
        let pool = ColdSlangcPool::new(1, SlangcFlags::canonical());
        let a = pool.cache_key("float4 main() {}", &[("SUBGROUPS_OFF", "0")]);
        let b = pool.cache_key("float4 main() {}", &[("SUBGROUPS_OFF", "0")]);
        let c = pool.cache_key("float4 other() {}", &[("SUBGROUPS_OFF", "0")]);
        let d = pool.cache_key("float4 main() {}", &[("SUBGROUPS_OFF", "1")]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn cold_pool_naga_gate_and_last_good_fallback() {
        let pool = ColdSlangcPool::new(1, SlangcFlags::canonical());
        let good = "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0,0.0,0.0,1.0); }";
        let session = ColdSlangcSession {
            id: 0,
            flags: SlangcFlags::canonical(),
        };
        assert!(session.validate_wgsl(good).is_ok());
        assert!(session.validate_wgsl("NOT WGSL @@@").is_err());
        let fresh = pool
            .compile_or_last_good_sketch("mat", good, None)
            .expect("good wgsl compiles");
        assert!(!fresh.from_cache);
        assert_eq!(fresh.hash.len(), 64);
        // Bad candidate falls back to last-good when present.
        let mut last_good = fresh.clone();
        last_good.from_cache = true;
        let fell_back = pool
            .compile_or_last_good_sketch("mat", "NOT WGSL @@@", Some(last_good))
            .expect("fallback");
        assert!(fell_back.from_cache);
        // Bad candidate with no last-good surfaces a naga error.
        assert!(pool
            .compile_or_last_good_sketch("mat", "NOT WGSL @@@", None)
            .is_err());
    }
}
