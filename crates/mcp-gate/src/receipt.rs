//! `receipt.json` writer: every staging dir carries its full provenance
//! (spec 05 §7). Replays never call a model: `sim.replay` consumes the
//! transcript + merged bytes, so long-term reproducibility needs only the
//! repo + `hashes.lock`.
//!
//! Content addressing is blake3 throughout (spec 00: blake3 everywhere),
//! including the `model_digest` slot — the OpenRAIL/CC-NC provenance that
//! L-4 cares about travels in `model_card_url` + `model_license`.

use serde::{Deserialize, Serialize};

/// Staging receipt (shape mirrored in `schemas/receipt.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub staging_hash: String,
    pub prompt_hash: String,
    pub model_digest: String,
    pub model_card_url: String,
    pub model_license: String,
    pub seed: u64,
    pub params: ReceiptParams,
    pub gates: String,
    pub approver: String,
    pub merged_hash: Option<String>,
    pub ai_tier: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReceiptParams {
    pub temperature: f32,
}

pub fn blake3_hex(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

pub fn write_receipt(staging_dir: &std::path::Path, receipt: &Receipt) -> std::io::Result<()> {
    std::fs::create_dir_all(staging_dir)?;
    let text = serde_json::to_string_pretty(receipt)
        .map(|mut s| {
            s.push('\n');
            s
        })
        .unwrap_or_else(|_| "{}".to_string());
    std::fs::write(staging_dir.join("receipt.json"), text)
}

pub fn read_receipt(staging_dir: &std::path::Path) -> Option<Receipt> {
    let text = std::fs::read_to_string(staging_dir.join("receipt.json")).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_round_trips_with_hashes() {
        let dir = std::env::temp_dir().join("mcp-gate-receipt");
        let _ = std::fs::remove_dir_all(&dir);
        let receipt = Receipt {
            staging_hash: blake3_hex(b"patch"),
            prompt_hash: blake3_hex(b"prompt"),
            model_digest: blake3_hex(b"model"),
            model_card_url: "https://example.invalid/card".to_string(),
            model_license: "Apache-2.0".to_string(),
            seed: 42,
            params: ReceiptParams { temperature: 0.0 },
            gates: "gate.report.json".to_string(),
            approver: "human:alice".to_string(),
            merged_hash: Some(blake3_hex(b"merged")),
            ai_tier: "local 8B Q4".to_string(),
        };
        write_receipt(&dir, &receipt).unwrap();
        let back = read_receipt(&dir).unwrap();
        assert_eq!(back.seed, 42);
        assert_eq!(back.approver, "human:alice");
        assert!(back.staging_hash.starts_with("blake3:"));
        assert!(back.prompt_hash.starts_with("blake3:"));
    }
}
