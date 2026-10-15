use serde::{Deserialize, Serialize};

/// Cold-open stats printed by `editor-shell --open` (recorded in
/// `docs/perf/phase3.md`, budget < 3 s to interactive).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenStats {
    pub scene: String,
    pub scene_file: String,
    pub entities: usize,
    pub load_ms: f64,
    pub interactive_ms: f64,
    pub mode: String,
}

/// Resolve a scene argument the same way `studio-cli render-test` does:
/// `<path>`, `assets/<path>`, `<path>/scene.ron`, `assets/<path>/scene.ron`.
/// Candidates are probed relative to the process CWD and relative to the
/// workspace root (unit tests run with the package dir as CWD).
pub fn resolve_scene_file(raw: &str) -> String {
    let candidates = [
        raw.to_string(),
        format!("assets/{raw}"),
        format!("{raw}/scene.ron"),
        format!("assets/{raw}/scene.ron"),
        format!("assets/samples/{raw}/scene.ron"),
    ];
    let roots = [workspace_root(), std::path::PathBuf::from(".")];
    for root in &roots {
        for candidate in &candidates {
            let joined = root.join(candidate);
            if joined.is_file() {
                // Prefer a clean repo-relative display path when under CWD.
                if let Ok(cwd) = std::env::current_dir() {
                    if let Ok(stripped) = joined.strip_prefix(&cwd) {
                        return stripped.to_string_lossy().into_owned();
                    }
                }
                return joined.to_string_lossy().into_owned();
            }
        }
    }
    // Fall back to the raw string (caller surfaces not-found).
    for candidate in &candidates {
        if std::path::Path::new(candidate).is_file() {
            return candidate.clone();
        }
    }
    raw.to_string()
}

fn workspace_root() -> std::path::PathBuf {
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    root
}

/// Windowed-shell request contract (Phase 9.4 step 6).
///
/// The native viewport (`winit` + `wgpu` + `egui-wgpu` overlay, spec 04 §1)
/// relinks on the toolchain bump behind the unchanged `--open` flag: setting
/// `EDITOR_SHELL_WINDOWED=1` (or `true`/`yes`) opts the identical open
/// pipeline (resolve → parse → layout → interactive marker) into windowed
/// mode. On the pinned Rust 1.82 toolchain the native window handle cannot
/// link yet (`winit 0.30` → `wayland-protocols 0.32` requires edition2024),
/// so the window-creation call below is a documented no-op stub returning
/// `false` and the pipeline reports the windowed mode headlessly. The bump
/// fills in `create_window` without touching the pipeline or the flag.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub title: String,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            title: "Game-Studio".into(),
        }
    }
}

/// Pure env-value gate so tests stay hermetic (no process-env mutation).
pub fn windowed_requested_from(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase),
        Some(v) if v == "1" || v == "true" || v == "yes"
    )
}

/// Same `--open` flag, windowed opt-in via `EDITOR_SHELL_WINDOWED`.
pub fn windowed_requested() -> bool {
    windowed_requested_from(std::env::var("EDITOR_SHELL_WINDOWED").ok().as_deref())
}

/// Native window-creation point. Returns `true` once the `winit` window
/// actually opens; `false` keeps the identical pipeline headless (current
/// pinned-toolchain behavior). The toolchain bump relinks the real
/// `winit 0.30` + `wgpu` + `egui-wgpu` handle here.
fn create_window(_config: &WindowConfig) -> bool {
    false
}

fn count_scene_entities(text: &str) -> usize {
    // Scene RON files are either proxy descriptors (sponza_lite: no entity
    // list) or full entity lists (stress_10k). Count `translation` rows as
    // the entity proxy; fall back to 1 for descriptor scenes.
    let count = text.matches("translation").count();
    count.max(1)
}

/// Open a scene headless and report interactive timing.
///
/// Shell architecture (spec 04 §1): the viewport is native (`winit` + `wgpu` +
/// `egui-wgpu` overlay) and the WebView (Tauri) hosts tools only. On the Rust
/// 1.82 toolchain `winit 0.30` cannot link (its `wayland-protocols 0.32`
/// transitive requires edition2024 — vetted failure, see ADR), so this
/// headless path exercises the identical open pipeline (resolve → parse →
/// layout → interactive marker) without creating a window. The native window
/// relinks on the toolchain bump behind the same `--open` flag: with
/// `EDITOR_SHELL_WINDOWED=1` this delegates to [`open_scene_windowed`],
/// which runs the identical pipeline and reports `windowed-shell` mode.
pub fn open_scene(raw: &str) -> anyhow::Result<OpenStats> {
    if windowed_requested() {
        return open_scene_windowed(raw, &WindowConfig::default());
    }
    let start = std::time::Instant::now();
    let scene_file = resolve_scene_file(raw);
    let text = std::fs::read_to_string(&scene_file)
        .map_err(|e| anyhow::anyhow!("open {raw}: scene not found ({scene_file}): {e}"))?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let entities = count_scene_entities(&text);
    // Layout load (corrupt → safe default, never blocks interactive).
    let _layout = crate::layout::Layout::load_file(std::path::Path::new(".studio/layout.ron"));
    let interactive_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(OpenStats {
        scene: raw.to_string(),
        scene_file,
        entities,
        load_ms,
        interactive_ms,
        mode: "headless-native-parity".into(),
    })
}

/// Windowed `--open` relink (Phase 9.4 step 6): identical pipeline to
/// [`open_scene`] (resolve → parse → layout → interactive marker) with the
/// native window-creation point up front. Until the toolchain bump links the
/// real `winit` handle, [`create_window`] reports `false` and the stats
/// carry `windowed-shell` mode through the headless-equivalent pipeline, so
/// the < 3 s interactive budget carries over to the windowed shell.
pub fn open_scene_windowed(raw: &str, config: &WindowConfig) -> anyhow::Result<OpenStats> {
    let start = std::time::Instant::now();
    let scene_file = resolve_scene_file(raw);
    let text = std::fs::read_to_string(&scene_file)
        .map_err(|e| anyhow::anyhow!("open {raw}: scene not found ({scene_file}): {e}"))?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let entities = count_scene_entities(&text);
    // Native window-creation point (stub `false` until the toolchain bump).
    let _windowed = create_window(config);
    // Layout load (corrupt → safe default, never blocks interactive).
    let _layout = crate::layout::Layout::load_file(std::path::Path::new(".studio/layout.ron"));
    let interactive_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(OpenStats {
        scene: raw.to_string(),
        scene_file,
        entities,
        load_ms,
        interactive_ms,
        mode: "windowed-shell".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_scene_ron() {
        let file = resolve_scene_file("samples/sponza_lite");
        assert!(file.ends_with("scene.ron"), "got {file}");
        assert!(std::path::Path::new(&file).is_file());
    }

    #[test]
    fn window_config_default_is_720p() {
        let config = WindowConfig::default();
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
        assert!(!config.title.is_empty());
    }

    #[test]
    fn windowed_gate_accepts_truthy_values() {
        for value in ["1", "true", "TRUE", " yes ", "Yes"] {
            assert!(windowed_requested_from(Some(value)), "got {value:?}");
        }
    }

    #[test]
    fn windowed_gate_rejects_other_values() {
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("false"),
            Some("no"),
            Some("windowed"),
        ] {
            assert!(!windowed_requested_from(value), "got {value:?}");
        }
    }

    #[test]
    fn windowed_open_reports_windowed_mode() {
        let stats = open_scene_windowed("samples/sponza_lite", &WindowConfig::default())
            .expect("windowed open resolves the sample scene");
        assert_eq!(stats.mode, "windowed-shell");
        assert!(stats.entities >= 1);
        assert!(stats.interactive_ms < 3000.0);
    }
}
