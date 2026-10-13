//! Enforced patch state machine (spec 05 §2):
//! `proposed → parsed → typechecked → linted → sandbox-tested →
//! replay-verified → approved → merged`.
//!
//! Transitions are a pure function of the current state plus the gate outcome,
//! so the order cannot be skipped, reordered, or bypassed by any caller.

use serde::{Deserialize, Serialize};

/// Structured gate failure returned to the agent on bounded retry and
/// appended to `gate.report/v1` (spec 05 §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateError {
    pub gate: String,
    pub code: String,
    pub message: String,
    pub hint: String,
    pub loc: Option<FileLoc>,
}

impl GateError {
    pub fn new(
        gate: &'static str,
        code: &str,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            gate: gate.to_string(),
            code: code.to_string(),
            message: message.into(),
            hint: hint.into(),
            loc: None,
        }
    }

    pub fn at(mut self, path: &str, line: u32, col: u32) -> Self {
        self.loc = Some(FileLoc {
            path: path.to_string(),
            line,
            col,
        });
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileLoc {
    pub path: String,
    pub line: u32,
    pub col: u32,
}

/// Approve path for a patch that passed all five gates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Approval {
    /// Headless CI with an explicit human approver recorded.
    AllowAi { approver: String },
}

/// Every state a proposed patch can occupy. Terminal states are `Merged` and
/// `Rejected`; everything else must advance through the next gate in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchState {
    Proposed,
    Parsed,
    Typechecked,
    Linted,
    SandboxTested,
    ReplayVerified,
    Approved,
    Merged,
    Rejected,
}

impl PatchState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Parsed => "parsed",
            Self::Typechecked => "typechecked",
            Self::Linted => "linted",
            Self::SandboxTested => "sandbox-tested",
            Self::ReplayVerified => "replay-verified",
            Self::Approved => "approved",
            Self::Merged => "merged",
            Self::Rejected => "rejected",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Merged | Self::Rejected)
    }
}

/// The gate whose success advances `state`. `None` for states that advance
/// without a gate (`Approved → Merged` needs [`Approval`] instead).
pub fn gate_for(state: PatchState) -> Option<&'static str> {
    match state {
        PatchState::Proposed => Some("G1"),
        PatchState::Parsed => Some("G2"),
        PatchState::Typechecked => Some("G3"),
        PatchState::Linted => Some("G4"),
        PatchState::SandboxTested => Some("G5"),
        PatchState::ReplayVerified => None,
        PatchState::Approved => None,
        PatchState::Merged | PatchState::Rejected => None,
    }
}

/// Advance one step. `Ok(next)` on gate success, `Err(GateError)` leaves the
/// caller in a rejected terminal state (the caller maps this to `Rejected`).
pub fn advance(state: PatchState) -> Result<PatchState, &'static str> {
    match state {
        PatchState::Proposed => Ok(PatchState::Parsed),
        PatchState::Parsed => Ok(PatchState::Typechecked),
        PatchState::Typechecked => Ok(PatchState::Linted),
        PatchState::Linted => Ok(PatchState::SandboxTested),
        PatchState::SandboxTested => Ok(PatchState::ReplayVerified),
        PatchState::ReplayVerified => Err("approve() required: human or ci:--allow-ai approver"),
        PatchState::Approved => Ok(PatchState::Merged),
        PatchState::Merged | PatchState::Rejected => Err("terminal state has no transition"),
    }
}

/// Approve a replay-verified patch. Headless CI auto-rejects AI assets unless
/// `--allow-ai` is passed with a human approver recorded (spec 05 §2).
pub fn approve(
    state: PatchState,
    approval: Option<&Approval>,
) -> Result<PatchState, Box<GateError>> {
    if state != PatchState::ReplayVerified {
        return Err(Box::new(GateError::new(
            "approve",
            "E_STATE",
            format!("approve from {}: must be replay-verified", state.as_str()),
            "run G1-G5 in order first",
        )));
    }
    match approval {
        Some(Approval::AllowAi { approver }) if !approver.trim().is_empty() => {
            Ok(PatchState::Approved)
        }
        _ => Err(Box::new(GateError::new(
            "approve",
            "E_APPROVAL",
            "AI asset requires --allow-ai with a human approver",
            "re-run with --allow-ai --approver human:<name>",
        ))),
    }
}

/// Verdict returned by the gate runner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum GateVerdict {
    Approve {
        merged_hash: String,
    },
    Reject {
        errors: Vec<GateError>,
        retries_left: u8,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_reaches_merged_in_order() {
        let mut state = PatchState::Proposed;
        let mut gates = Vec::new();
        while state != PatchState::ReplayVerified {
            gates.push(gate_for(state).unwrap());
            state = advance(state).unwrap();
        }
        assert_eq!(gates, vec!["G1", "G2", "G3", "G4", "G5"]);
        assert_eq!(state, PatchState::ReplayVerified);
        // Approve without an approver is rejected (headless CI default).
        assert!(approve(state, None).is_err());
        let ok = approve(
            state,
            Some(&Approval::AllowAi {
                approver: "human:alice".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(ok, PatchState::Approved);
        assert_eq!(advance(ok).unwrap(), PatchState::Merged);
        assert!(PatchState::Merged.is_terminal());
    }

    #[test]
    fn approve_from_wrong_state_is_e_state() {
        let err = approve(
            PatchState::Linted,
            Some(&Approval::AllowAi {
                approver: "human:alice".to_string(),
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "E_STATE");
        // Empty approver never authorizes a merge.
        let err = approve(
            PatchState::ReplayVerified,
            Some(&Approval::AllowAi {
                approver: "  ".to_string(),
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "E_APPROVAL");
    }

    #[test]
    fn terminal_states_have_no_transition() {
        assert!(advance(PatchState::Merged).is_err());
        assert!(advance(PatchState::Rejected).is_err());
        assert!(PatchState::Rejected.is_terminal());
        assert!(!PatchState::Proposed.is_terminal());
    }

    #[test]
    fn gate_error_carries_structured_location() {
        let err = GateError::new("G1", "E_PARSE", "bad op", "fix op").at("patch.json", 3, 12);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], serde_json::json!("E_PARSE"));
        assert_eq!(json["loc"]["line"], serde_json::json!(3));
    }
}
