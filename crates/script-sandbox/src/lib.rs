pub const DEFAULT_FUEL: u64 = 10_000_000;
pub const DEFAULT_MEM_MIB: u32 = 64;
pub const DEFAULT_EPOCH_MS: u64 = 50;

pub fn name() -> &'static str {
    "script-sandbox"
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_defaults_match_spec() {
        assert_eq!(DEFAULT_FUEL, 10_000_000);
        assert_eq!(DEFAULT_MEM_MIB, 64);
        assert_eq!(DEFAULT_EPOCH_MS, 50);
        assert!(!version().is_empty());
    }
}
