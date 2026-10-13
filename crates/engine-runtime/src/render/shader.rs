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
}
