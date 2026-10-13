use serde::{Deserialize, Serialize};

/// Canonical `asset.changed/v1` notification (spec 04 §2 + §6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetChanged {
    pub handle: String,
    pub old_hash: String,
    pub new_hash: String,
}

impl AssetChanged {
    pub fn new(handle: &str, old_hash: &str, new_hash: &str) -> Self {
        Self {
            handle: handle.to_string(),
            old_hash: old_hash.to_string(),
            new_hash: new_hash.to_string(),
        }
    }

    pub fn valid(&self) -> bool {
        !self.handle.is_empty()
            && self.old_hash.starts_with("blake3:")
            && self.new_hash.starts_with("blake3:")
    }

    /// Render as a JSON-RPC 2.0 notification (no `id`).
    pub fn to_notification(&self) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "asset.changed/v1",
            "params": {
                "handle": self.handle,
                "old_hash": self.old_hash,
                "new_hash": self.new_hash,
            },
        })
        .to_string()
    }
}

/// Canonical `gate.report/v1` event envelope (owned by Phase 4 gates;
/// the shape is frozen here so Phase-3 history can already emit it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateReport {
    pub verdict: String,
    pub hash: String,
}

impl GateReport {
    pub fn pass(hash: &str) -> Self {
        Self {
            verdict: "pass".to_string(),
            hash: hash.to_string(),
        }
    }

    pub fn to_notification(&self) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "gate.report/v1",
            "params": { "verdict": self.verdict, "hash": self.hash },
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_notification_round_trip() {
        let h = format!(
            "blake3:{}",
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        let event = AssetChanged::new("tex:albedo#7", &h, &h);
        assert!(event.valid());
        let text = event.to_notification();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["method"], serde_json::json!("asset.changed/v1"));
        assert!(value.get("id").is_none());
        assert_eq!(value["params"]["handle"], serde_json::json!("tex:albedo#7"));
    }

    #[test]
    fn changed_rejects_bad_hashes() {
        let event = AssetChanged::new("tex:a#1", "nope", "blake3:also-nope");
        assert!(!event.valid());
    }
}
