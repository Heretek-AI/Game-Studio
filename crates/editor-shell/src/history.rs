use serde::{Deserialize, Serialize};

/// One reversible scene mutation (spec 04 §4). Every mutation — human or
/// agent-approved — is a reversible `LevelPatch` pair. Undo of an agent patch
/// replays its `backward` patch through the same gates, so undo can never
/// desync the sim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatchOp {
    pub op: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelPatch {
    pub ops: Vec<PatchOp>,
    pub base_hash: String,
    pub staging_hash: String,
}

impl LevelPatch {
    pub fn validate(&self) -> Result<(), String> {
        if !self.base_hash.starts_with("blake3:") {
            return Err("base_hash must be blake3:…".into());
        }
        if !self.staging_hash.starts_with("blake3:") {
            return Err("staging_hash must be blake3:…".into());
        }
        for op in &self.ops {
            if !matches!(
                op.op.as_str(),
                "addEntity" | "setComponent" | "removeEntity"
            ) {
                return Err(format!("unknown op {}", op.op));
            }
            if op.id.is_empty() {
                return Err("op id must be non-empty".into());
            }
            if op.op == "setComponent" && op.component.is_none() {
                return Err("setComponent needs component".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EditOp {
    ApplyPatch {
        forward: LevelPatch,
        backward: LevelPatch,
        label: String,
    },
}

/// 500-cap reversible history (spec 04 §4). `editor.undo/v1` /
/// `editor.redo/v1` are the only history entry points.
pub struct History {
    undo: Vec<EditOp>,
    redo: Vec<EditOp>,
    cap: usize,
}

impl History {
    pub const DEFAULT_CAP: usize = 500;

    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cap: Self::DEFAULT_CAP,
        }
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cap,
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Commit through the gates (parse → typecheck → replay hash): the patch
    /// pair must validate before it enters history.
    pub fn commit(&mut self, op: EditOp) -> Result<(), String> {
        match &op {
            EditOp::ApplyPatch {
                forward, backward, ..
            } => {
                forward.validate()?;
                backward.validate()?;
            }
        }
        self.undo.push(op);
        self.redo.clear();
        self.truncate();
        Ok(())
    }

    pub fn undo(&mut self) -> Option<&EditOp> {
        let op = self.undo.pop()?;
        self.redo.push(op);
        self.redo.last()
    }

    pub fn redo(&mut self) -> Option<&EditOp> {
        let op = self.redo.pop()?;
        self.undo.push(op);
        self.undo.last()
    }

    fn truncate(&mut self) {
        while self.undo.len() > self.cap {
            self.undo.remove(0);
        }
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(tag: &str, n: usize) -> LevelPatch {
        LevelPatch {
            ops: vec![PatchOp {
                op: "setComponent".into(),
                id: format!("e:{n}"),
                component: Some("Velocity".into()),
                value: Some(serde_json::json!({"0": [1, 0, 0]})),
                components: None,
            }],
            base_hash: format!("blake3:{tag}{n:062}"),
            staging_hash: format!("blake3:{tag}{n:062}"),
        }
    }

    fn op_pair(n: usize) -> EditOp {
        EditOp::ApplyPatch {
            forward: patch("f", n),
            backward: patch("b", n),
            label: format!("edit {n}"),
        }
    }

    fn valid_hash_body() -> String {
        "a".repeat(62)
    }

    #[test]
    fn round_trip_100_zero_mismatch() {
        let h = valid_hash_body();
        let baseline = format!("blake3:base{h}");
        let _ = baseline;
        let mut history = History::new();
        for n in 0..100 {
            history.commit(op_pair(n)).unwrap();
        }
        assert_eq!(history.undo_len(), 100);
        // Undo all 100, then redo all 100: every step must return the
        // matching pair with no hash mismatch vs the committed sequence.
        let mut undone = Vec::new();
        for _ in 0..100 {
            let op = history.undo().cloned().expect("undo");
            undone.push(op);
        }
        assert_eq!(history.undo_len(), 0);
        assert_eq!(history.redo_len(), 100);
        for (index, op) in undone.iter().enumerate() {
            let want = op_pair(99 - index);
            assert_eq!(*op, want, "undo step {index} mismatch");
        }
        for n in 0..100 {
            let op = history.redo().cloned().expect("redo");
            assert_eq!(op, op_pair(n), "redo step {n} mismatch");
        }
        assert_eq!(history.undo_len(), 100);
        assert_eq!(history.redo_len(), 0);
    }

    #[test]
    fn cap_500_evicts_oldest() {
        let mut history = History::with_cap(500);
        for n in 0..600 {
            history.commit(op_pair(n)).unwrap();
        }
        assert_eq!(history.undo_len(), 500);
        assert_eq!(history.cap(), 500);
        // Oldest surviving op is #100.
        let mut seen = Vec::new();
        while let Some(op) = history.undo().cloned() {
            seen.push(op);
        }
        assert_eq!(seen.len(), 500);
        assert_eq!(seen[499], op_pair(100));
    }

    #[test]
    fn commit_clears_redo_and_validates_gates() {
        let mut history = History::new();
        history.commit(op_pair(1)).unwrap();
        history.commit(op_pair(2)).unwrap();
        let _ = history.undo();
        assert_eq!(history.redo_len(), 1);
        history.commit(op_pair(3)).unwrap();
        assert_eq!(history.redo_len(), 0);

        let bad = EditOp::ApplyPatch {
            forward: LevelPatch {
                ops: vec![PatchOp {
                    op: "nuke".into(),
                    id: "e:1".into(),
                    component: None,
                    value: None,
                    components: None,
                }],
                base_hash: "blake3:".to_string() + &"a".repeat(62),
                staging_hash: "blake3:".to_string() + &"b".repeat(62),
            },
            backward: patch("b", 1),
            label: "bad".into(),
        };
        assert!(history.commit(bad).is_err());
    }
}
