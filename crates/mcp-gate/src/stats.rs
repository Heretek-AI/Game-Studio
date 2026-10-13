//! Hallucination-reject rate: fraction of agent proposals rejected by gates,
//! with per-gate breakdown (spec 05 §4, red-team R-13). Reported, not hidden.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-gate counters for one reporting window.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GateStats {
    pub proposed: u64,
    pub rejected: BTreeMap<String, u64>,
}

impl GateStats {
    pub fn record_proposal(&mut self) {
        self.proposed += 1;
    }

    pub fn record_reject(&mut self, gate: &str) {
        self.proposed += 1;
        *self.rejected.entry(gate.to_string()).or_insert(0) += 1;
    }

    pub fn reject_rate(&self) -> f64 {
        if self.proposed == 0 {
            return 0.0;
        }
        let total: u64 = self.rejected.values().sum();
        total as f64 / self.proposed as f64
    }

    /// Markdown table rows: `| gate | rejects | rate |` + a total row.
    pub fn table(&self) -> String {
        let mut out = String::from("| gate | rejects | share of proposals |\n|---|---|---|\n");
        for gate in ["G1", "G2", "G3", "G4", "G5"] {
            let n = self.rejected.get(gate).copied().unwrap_or(0);
            let share = if self.proposed == 0 {
                0.0
            } else {
                n as f64 / self.proposed as f64 * 100.0
            };
            out.push_str(&format!("| {gate} | {n} | {share:.1}% |\n"));
        }
        out.push_str(&format!(
            "| TOTAL | {} | {:.1}% |\n",
            self.rejected.values().sum::<u64>(),
            self.reject_rate() * 100.0
        ));
        out
    }
}

/// Append one JSONL outcome row to the local history file. History is local
/// telemetry (gitignored); the report is additionally reproducible from the
/// deterministic evaluation set, so CI never depends on this file existing.
pub fn append_outcome(path: &std::path::Path, gate: Option<&str>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    use std::io::Write as _;
    let row = serde_json::json!({
        "gate": gate,
        "rejected": gate.is_some(),
    });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{row}")?;
    Ok(())
}

/// Load history rows; missing file means no history (not an error).
pub fn load_stats(path: &std::path::Path) -> GateStats {
    let mut stats = GateStats::default();
    let text = std::fs::read_to_string(path).unwrap_or_default();
    for line in text.lines() {
        let row: serde_json::Value = match serde_json::from_str(line.trim()) {
            Ok(row) => row,
            Err(_) => continue,
        };
        match row.get("gate").and_then(|g| g.as_str()) {
            Some(gate) => stats.record_reject(gate),
            None => stats.record_proposal(),
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_rate_with_per_gate_breakdown() {
        let mut stats = GateStats::default();
        stats.record_proposal();
        stats.record_reject("G2");
        stats.record_reject("G2");
        stats.record_reject("G5");
        assert_eq!(stats.proposed, 4);
        assert!((stats.reject_rate() - 0.75).abs() < 1e-9);
        let table = stats.table();
        assert!(table.contains("| G2 | 2 | 50.0% |"));
        assert!(table.contains("| G5 | 1 | 25.0% |"));
        assert!(table.contains("| TOTAL | 3 | 75.0% |"));
    }

    #[test]
    fn history_round_trips_and_missing_file_is_empty() {
        let dir = std::env::temp_dir().join("mcp-gate-stats");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.jsonl");
        assert_eq!(load_stats(&path).proposed, 0);
        append_outcome(&path, None).unwrap();
        append_outcome(&path, Some("G1")).unwrap();
        let stats = load_stats(&path);
        assert_eq!(stats.proposed, 2);
        assert_eq!(stats.rejected.get("G1"), Some(&1));
    }
}
