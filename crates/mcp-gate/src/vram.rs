//! VRAM/RAM budget tiers (spec 05 §6, red-team R-12).
//!
//! Policy: if the render floor cannot be met with local AI active, **AI
//! features degrade first** (smaller model → CPU → off), never texture
//! resolution silently. The active tier is surfaced (`AI: local 8B Q4 /
//! local 3B / CPU / cloud / off`) with one click to change.

use serde::{Deserialize, Serialize};

/// Render floor: 6–8 GiB reserved, hard floor (spec 05 §6).
pub const RENDER_FLOOR_GIB: u64 = 8;
/// 7–8B Q4 weights need 5–7 GiB.
pub const MODEL_8B_GIB: u64 = 7;
/// 3B Q4 weights need ~3–4 GiB.
pub const MODEL_3B_GIB: u64 = 4;
/// KV-cache + compute scratch at reduced ctx: ~2 GiB.
pub const MODEL_CPU_GIB: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiTier {
    Local8bQ4,
    Local3B,
    Cpu,
    Cloud,
    Off,
}

impl AiTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local8bQ4 => "local 8B Q4",
            Self::Local3B => "local 3B",
            Self::Cpu => "CPU",
            Self::Cloud => "cloud",
            Self::Off => "off",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "local 8b q4" | "8b" | "local-8b" => Some(Self::Local8bQ4),
            "local 3b" | "3b" | "local-3b" => Some(Self::Local3B),
            "cpu" => Some(Self::Cpu),
            "cloud" => Some(Self::Cloud),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// One degradation step toward `Off`. Render floor first: AI degrades,
    /// never texture resolution.
    pub fn degrade(self) -> Self {
        match self {
            Self::Local8bQ4 => Self::Local3B,
            Self::Local3B => Self::Cpu,
            Self::Cpu => Self::Cloud,
            Self::Cloud => Self::Off,
            Self::Off => Self::Off,
        }
    }
}

/// Render-floor-first tier selection: reserve the render floor, then fit the
/// largest model tier into what remains.
pub fn select_tier(total_gib: u64) -> AiTier {
    let spare = total_gib.saturating_sub(RENDER_FLOOR_GIB);
    if spare >= MODEL_8B_GIB {
        AiTier::Local8bQ4
    } else if spare >= MODEL_3B_GIB {
        AiTier::Local3B
    } else if spare >= MODEL_CPU_GIB {
        AiTier::Cpu
    } else if spare >= 1 {
        // No local budget left: cloud BYOK keeps AI available with zero
        // VRAM, or the operator switches fully off.
        AiTier::Cloud
    } else {
        AiTier::Off
    }
}

/// Read total system RAM in GiB from /proc/meminfo (Linux CI/dev).
pub fn system_ram_gib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb / 1024 / 1024);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_floor_wins_degradation_order() {
        assert_eq!(select_tier(16), AiTier::Local8bQ4);
        assert_eq!(select_tier(12), AiTier::Local3B);
        assert_eq!(select_tier(10), AiTier::Cpu);
        assert_eq!(select_tier(9), AiTier::Cloud);
        assert_eq!(select_tier(8), AiTier::Off);
        assert_eq!(select_tier(0), AiTier::Off);
    }

    #[test]
    fn degrade_steps_down_to_off() {
        let mut tier = AiTier::Local8bQ4;
        let mut seen = vec![tier.label()];
        for _ in 0..5 {
            tier = tier.degrade();
            seen.push(tier.label());
        }
        assert_eq!(
            seen,
            vec!["local 8B Q4", "local 3B", "CPU", "cloud", "off", "off"]
        );
    }

    #[test]
    fn tier_labels_match_acceptance_surface() {
        assert_eq!(AiTier::Local8bQ4.label(), "local 8B Q4");
        assert_eq!(AiTier::Local3B.label(), "local 3B");
        assert_eq!(AiTier::Off.label(), "off");
        assert!(AiTier::parse("local 8B Q4").is_some());
        assert!(AiTier::parse("nonsense").is_none());
    }
}
