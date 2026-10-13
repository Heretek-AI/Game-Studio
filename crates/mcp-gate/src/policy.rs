//! Agent budgets and append-only transcripts (spec 05 §1–§2).
//!
//! Every agent call carries a [`CallBudget`] (tokens, wall-clock, retries,
//! scratch bytes). Every prompt, tool call, and gate verdict is appended to a
//! [`TranscriptLog`] (JSONL). The log is append-only by construction: the only
//! write API is [`TranscriptLog::append`], which opens the file with
//! `.append(true)` and never truncates.

use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Agent roster from spec 05 §1. Unknown strings are rejected at parse time so
/// a typo can never silently widen tool access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentId {
    SceneBuilder,
    ShadingVfx,
    LogicScripting,
    QaPlaytester,
}

impl AgentId {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "scene_builder" => Some(Self::SceneBuilder),
            "shading_vfx" => Some(Self::ShadingVfx),
            "logic_scripting" => Some(Self::LogicScripting),
            "qa_playtester" => Some(Self::QaPlaytester),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SceneBuilder => "scene_builder",
            Self::ShadingVfx => "shading_vfx",
            Self::LogicScripting => "logic_scripting",
            Self::QaPlaytester => "qa_playtester",
        }
    }
}

/// Bounded retries, token budget, wall-clock timeout, scratch cap
/// (spec 05 §1: defaults 3 / 2048 / 8 s per call / 5 min per task).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallBudget {
    pub max_tokens: u32,
    pub timeout_ms: u64,
    pub retries: u8,
    pub max_scratch_bytes: u64,
}

impl Default for CallBudget {
    fn default() -> Self {
        Self {
            max_tokens: 2048,
            timeout_ms: 8000,
            retries: 3,
            max_scratch_bytes: 4 << 20,
        }
    }
}

/// Per-call context for one agent task (spec 05 §2).
pub struct AgentCtx {
    pub agent: AgentId,
    pub task_id: String,
    pub staging: PathBuf,
    pub budget: CallBudget,
    pub transcript: TranscriptLog,
    pub tokens_used: u32,
    pub retries_left: u8,
    pub deadline: Instant,
    pub scratch_bytes: u64,
}

impl AgentCtx {
    pub fn new(
        agent: AgentId,
        task_id: String,
        staging: PathBuf,
        budget: CallBudget,
        transcript: TranscriptLog,
    ) -> Self {
        let deadline = Instant::now() + Duration::from_millis(budget.timeout_ms.max(1));
        let retries_left = budget.retries;
        Self {
            agent,
            task_id,
            staging,
            budget,
            transcript,
            tokens_used: 0,
            retries_left,
            deadline,
            scratch_bytes: 0,
        }
    }

    /// Charge completion tokens; fails closed once the budget is exhausted.
    pub fn charge_tokens(&mut self, tokens: u32) -> Result<(), String> {
        self.tokens_used = self.tokens_used.saturating_add(tokens);
        if self.tokens_used > self.budget.max_tokens {
            return Err(format!(
                "E_BUDGET tokens {}/{} exhausted",
                self.tokens_used, self.budget.max_tokens
            ));
        }
        Ok(())
    }

    /// Fail closed once the wall-clock budget is exhausted.
    pub fn check_deadline(&self) -> Result<(), String> {
        if Instant::now() > self.deadline {
            return Err("E_BUDGET wall-clock timeout exhausted".to_string());
        }
        Ok(())
    }

    /// Charge staging scratch bytes; fails closed past the cap.
    pub fn charge_scratch(&mut self, bytes: u64) -> Result<(), String> {
        self.scratch_bytes = self.scratch_bytes.saturating_add(bytes);
        if self.scratch_bytes > self.budget.max_scratch_bytes {
            return Err(format!(
                "E_BUDGET scratch {}/{} bytes exhausted",
                self.scratch_bytes, self.budget.max_scratch_bytes
            ));
        }
        Ok(())
    }

    /// Consume one bounded retry. Returns the remaining count, or an error
    /// when the retry budget is already spent.
    pub fn consume_retry(&mut self) -> Result<u8, String> {
        if self.retries_left == 0 {
            return Err("E_BUDGET retry budget exhausted".to_string());
        }
        self.retries_left -= 1;
        Ok(self.retries_left)
    }
}

/// One transcript row. `prev_hash` chains rows so truncation or reordering is
/// detectable without trusting the writer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub seq: u64,
    pub kind: String,
    pub task_id: String,
    pub payload_hash: String,
    pub prev_hash: String,
}

/// Append-only JSONL transcript (`prompt_hash + model_digest + seed +
/// output_hash` retention, spec 05 §1).
#[derive(Clone, Debug)]
pub struct TranscriptLog {
    path: PathBuf,
    task_id: String,
    next_seq: u64,
    head_hash: String,
}

impl TranscriptLog {
    pub fn create(path: PathBuf, task_id: &str) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        // Create without truncating an existing log: a transcript must never
        // lose rows, even across process restarts on the same task.
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        // Resume the chain when the staging dir already holds rows (same
        // content-addressed patch re-run, e.g. a bounded retry): continue
        // sequence numbers and link to the last row, never restart at
        // genesis.
        let mut log = Self {
            path,
            task_id: task_id.to_string(),
            next_seq: 0,
            head_hash: "blake3:genesis".to_string(),
        };
        for row in read_transcript(&log.path) {
            log.next_seq = log.next_seq.max(row.seq + 1);
            let raw = format!("{}:{}:{}", row.seq, row.kind, row.payload_hash);
            log.head_hash = format!("blake3:{}", blake3::hash(raw.as_bytes()).to_hex());
        }
        Ok(log)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Append one row. There is deliberately no edit, truncate, or rewrite
    /// API — retention is structural, not advisory.
    pub fn append(&mut self, kind: &str, payload_hash: &str) -> std::io::Result<TranscriptEntry> {
        let entry = TranscriptEntry {
            seq: self.next_seq,
            kind: kind.to_string(),
            task_id: self.task_id.clone(),
            payload_hash: payload_hash.to_string(),
            prev_hash: self.head_hash.clone(),
        };
        let line = serde_json::to_string(&entry).unwrap_or_else(|_| "{}".to_string());
        {
            let mut file = std::fs::OpenOptions::new().append(true).open(&self.path)?;
            writeln!(file, "{line}")?;
            file.flush()?;
        }
        self.next_seq += 1;
        let raw = format!("{}:{kind}:{payload_hash}", entry.seq);
        self.head_hash = format!("blake3:{}", blake3::hash(raw.as_bytes()).to_hex());
        Ok(entry)
    }

    pub fn len(&self) -> u64 {
        self.next_seq
    }

    pub fn is_empty(&self) -> bool {
        self.next_seq == 0
    }
}

/// Read back transcript rows in file order.
pub fn read_transcript(path: &std::path::Path) -> Vec<TranscriptEntry> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|line| serde_json::from_str(line.trim()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mcp-gate-policy-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn budget_enforcement_tokens_timeout_scratch() {
        let dir = scratch_dir("budget");
        let log = TranscriptLog::create(dir.join("t.jsonl"), "task:test").unwrap();
        let mut ctx = AgentCtx::new(
            AgentId::SceneBuilder,
            "task:test".to_string(),
            dir,
            CallBudget {
                max_tokens: 10,
                timeout_ms: 60_000,
                retries: 2,
                max_scratch_bytes: 8,
            },
            log,
        );
        assert!(ctx.charge_tokens(6).is_ok());
        assert!(ctx.charge_tokens(4).is_ok());
        assert!(ctx.charge_tokens(1).is_err());
        assert!(ctx.check_deadline().is_ok());
        assert!(ctx.charge_scratch(8).is_ok());
        assert!(ctx.charge_scratch(1).is_err());
    }

    #[test]
    fn retry_budget_decrements_and_fails_closed() {
        let dir = scratch_dir("retry");
        let log = TranscriptLog::create(dir.join("t.jsonl"), "task:test").unwrap();
        let mut ctx = AgentCtx::new(
            AgentId::QaPlaytester,
            "task:retry".to_string(),
            dir,
            CallBudget::default(),
            log,
        );
        assert_eq!(ctx.consume_retry().unwrap(), 2);
        assert_eq!(ctx.consume_retry().unwrap(), 1);
        assert_eq!(ctx.consume_retry().unwrap(), 0);
        assert!(ctx.consume_retry().is_err());
    }

    #[test]
    fn agent_ids_reject_unknown() {
        assert_eq!(AgentId::parse("scene_builder"), Some(AgentId::SceneBuilder));
        assert_eq!(AgentId::parse("qa_playtester"), Some(AgentId::QaPlaytester));
        assert_eq!(AgentId::parse("root"), None);
        assert_eq!(AgentId::parse(""), None);
    }

    #[test]
    fn transcript_append_only_preserves_prefix() {
        let dir = scratch_dir("append");
        let path = dir.join("t.jsonl");
        let mut log = TranscriptLog::create(path.clone(), "task:append").unwrap();
        log.append("prompt", "blake3:aaa").unwrap();
        log.append("tool_call", "blake3:bbb").unwrap();
        let before = std::fs::read(&path).unwrap();
        log.append("gate_verdict", "blake3:ccc").unwrap();
        let after = std::fs::read(&path).unwrap();
        // Append-only: previously written bytes are a strict prefix.
        assert!(after.starts_with(&before));
        assert_eq!(log.len(), 3);
        let rows = read_transcript(&path);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, "prompt");
        assert_eq!(rows[1].kind, "tool_call");
        assert_eq!(rows[2].kind, "gate_verdict");
        assert_eq!(rows[0].seq, 0);
        assert_eq!(rows[2].seq, 2);
        // Hash chain links every row to its predecessor.
        assert!(rows[1].prev_hash.len() > 7);
        assert_ne!(rows[1].prev_hash, rows[2].prev_hash);
        assert!(rows.iter().all(|row| row.task_id == "task:append"));
    }

    #[test]
    fn transcript_resumes_chain_across_restarts() {
        let dir = scratch_dir("resume");
        let path = dir.join("t.jsonl");
        {
            let mut log = TranscriptLog::create(path.clone(), "task:retry-1").unwrap();
            log.append("prompt", "blake3:aaa").unwrap();
            log.append("gate_verdict", "blake3:bbb").unwrap();
        }
        // Same content-addressed staging dir, new process (bounded retry):
        // sequence continues and links to the last row, never restarts.
        let mut log = TranscriptLog::create(path.clone(), "task:retry-1").unwrap();
        assert_eq!(log.len(), 2);
        let row = log.append("gate_verdict", "blake3:ccc").unwrap();
        assert_eq!(row.seq, 2);
        let rows = read_transcript(&path);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].seq, 2);
        let raw = format!("{}:{}:{}", rows[1].seq, rows[1].kind, rows[1].payload_hash);
        let want_prev = format!("blake3:{}", blake3::hash(raw.as_bytes()).to_hex());
        assert_eq!(rows[2].prev_hash, want_prev);
    }
}
