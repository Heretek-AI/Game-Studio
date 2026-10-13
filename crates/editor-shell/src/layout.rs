use serde::{Deserialize, Serialize};

/// Dock layout (spec 04 §5): `egui_dock` tree with the viewport always
/// center, plus left scene tree, right inspector, bottom browser/console/
/// timeline, and floating node/curve/MCP panels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub viewport_center: bool,
    pub left: String,
    pub right: String,
    pub bottom: String,
    pub tabs: Vec<String>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            viewport_center: true,
            left: "scene_tree".into(),
            right: "inspector".into(),
            bottom: "asset_browser,console,timeline".into(),
            tabs: vec![
                "viewport".into(),
                "scene_tree".into(),
                "inspector".into(),
                "asset_browser".into(),
                "console".into(),
                "node_graph".into(),
                "curve_editor".into(),
            ],
        }
    }
}

impl Layout {
    /// Load from RON text. Corrupt layout → safe default, never a crash
    /// (spec 04 §5 acceptance: layout-corrupt fallback).
    pub fn load_or_default(text: &str) -> Self {
        ron::from_str::<Layout>(text)
            .ok()
            .filter(|l| l.viewport_center)
            .unwrap_or_default()
    }

    pub fn load_file(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::load_or_default(&text))
            .unwrap_or_default()
    }

    pub fn save_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_layout_falls_back_to_safe_default() {
        for bad in [
            "",
            "not ron at all ((((",
            "(viewport_center: false, left: \"x\", right: \"y\", bottom: \"z\", tabs: [])",
            "(viewport_center: true, left: 42)",
        ] {
            let layout = Layout::load_or_default(bad);
            assert_eq!(
                layout,
                Layout::default(),
                "bad input {bad:?} must fall back"
            );
            assert!(layout.viewport_center);
        }
    }

    #[test]
    fn round_trip_file() {
        let dir = std::env::temp_dir().join(format!(
            "layout-rt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("layout.ron");
        let layout = Layout::default();
        layout.save_file(&path).unwrap();
        assert_eq!(Layout::load_file(&path), layout);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
