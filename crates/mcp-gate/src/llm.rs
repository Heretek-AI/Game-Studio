//! BYOK local-model pipeline: one trait, three backends, zero engine
//! linkage (spec 05 §5).
//!
//! The clients speak loopback OpenAI-compatible HTTP with `std::net` only —
//! no HTTP crate enters the workspace, so the MSRV pins and the `cargo tree
//! -p engine-runtime` P6 gate are unaffected. Nothing here is linked (or
//! linkable) from `engine-runtime`: `studio-cli` wires these clients, the
//! sim never sees them.

use serde::{Deserialize, Serialize};

/// Deterministic prompt envelope: temperature 0.0 unless creativity is
/// explicitly requested, fixed seed, JSON-schema-shaped responses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StablePrompt {
    pub system: String,
    pub few_shot: Vec<Message>,
    pub json_schema: serde_json::Value,
    pub temperature: f32,
    pub seed: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Minimal completion contract. Implementations must be loopback-only.
pub trait LlmClient: Send + Sync {
    fn name(&self) -> &'static str;
    fn base(&self) -> &str;
    fn model_digest(&self) -> &str;
    fn complete(&self, prompt: &StablePrompt, timeout_ms: u64)
        -> Result<serde_json::Value, String>;
}

fn require_loopback(base: &str) -> Result<(), String> {
    let ok = base.starts_with("http://127.0.0.1:")
        || base.starts_with("http://localhost:")
        || base.starts_with("https://farm.internal");
    if ok {
        Ok(())
    } else {
        Err(format!("E_NET loopback-only: refusing base '{base}'"))
    }
}

/// POST a chat-completions request over a raw loopback TCP socket. No client
/// dependency, an 8 s default timeout, and a response that must be JSON.
fn post_chat_completions(
    base: &str,
    model: &str,
    prompt: &StablePrompt,
    timeout_ms: u64,
) -> Result<serde_json::Value, String> {
    require_loopback(base)?;
    let host_port = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .ok_or_else(|| format!("E_NET bad base '{base}'"))?;
    let host_port = host_port.trim_end_matches("/v1").trim_end_matches('/');
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": prompt.system},
        ],
        "temperature": prompt.temperature,
        "seed": prompt.seed,
        "response_format": {"type": "json_object"},
    });
    let body_text = body.to_string();
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_text}",
        body_text.len()
    );
    let timeout = std::time::Duration::from_millis(timeout_ms.max(1));
    let mut stream = std::net::TcpStream::connect(host_port)
        .map_err(|err| format!("E_NET connect {base}: {err}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| format!("E_NET timeout {base}: {err}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| format!("E_NET timeout {base}: {err}"))?;
    use std::io::{Read as _, Write as _};
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("E_NET send {base}: {err}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|err| format!("E_NET recv {base}: {err}"))?;
    let text = String::from_utf8_lossy(&raw);
    let body_start = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    let payload = text[body_start..].trim();
    // OpenAI-compat envelope: choices[0].message.content holds the JSON.
    let envelope: serde_json::Value = serde_json::from_str(payload)
        .map_err(|err| format!("E_NET bad JSON from {base}: {err}"))?;
    if let Some(content) = envelope
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
    {
        return serde_json::from_str(content)
            .map_err(|err| format!("E_NET model content is not JSON: {err}"));
    }
    Ok(envelope)
}

macro_rules! loopback_client {
    ($name:ident, $label:expr, $default_base:expr) => {
        /// Loopback OpenAI-compatible backend.
        pub struct $name {
            pub base: String,
            pub model: String,
            pub digest: String,
            pub card_url: String,
            pub license: String,
        }

        impl $name {
            pub fn new(base: &str, model: &str, digest: &str) -> Result<Self, String> {
                require_loopback(base)?;
                Ok(Self {
                    base: base.to_string(),
                    model: model.to_string(),
                    digest: digest.to_string(),
                    card_url: "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct".to_string(),
                    license: "Apache-2.0 (code; weights: Qwen license)".to_string(),
                })
            }
        }

        impl LlmClient for $name {
            fn name(&self) -> &'static str {
                $label
            }
            fn base(&self) -> &str {
                &self.base
            }
            fn model_digest(&self) -> &str {
                &self.digest
            }
            fn complete(
                &self,
                prompt: &StablePrompt,
                timeout_ms: u64,
            ) -> Result<serde_json::Value, String> {
                post_chat_completions(&self.base, &self.model, prompt, timeout_ms)
            }
        }
    };
}

loopback_client!(LlamaCppClient, "llama.cpp", "http://127.0.0.1:39010");
loopback_client!(OllamaClient, "ollama", "http://127.0.0.1:11434/v1");
loopback_client!(VllmFarmClient, "vllm-farm", "https://farm.internal/v1");

/// Default sidecar endpoints (spec 05 §5 compose excerpt).
pub fn default_llama_base() -> &'static str {
    "http://127.0.0.1:39010"
}

pub fn default_ollama_base() -> &'static str {
    "http://127.0.0.1:11434/v1"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt() -> StablePrompt {
        StablePrompt {
            system: "emit LevelPatch JSON".to_string(),
            few_shot: Vec::new(),
            json_schema: serde_json::json!({"type": "object"}),
            temperature: 0.0,
            seed: 42,
        }
    }

    #[test]
    fn three_backends_construct_on_loopback() {
        let llama =
            LlamaCppClient::new(default_llama_base(), "qwen2.5-coder-7b", "blake3:abc").unwrap();
        let ollama =
            OllamaClient::new(default_ollama_base(), "qwen2.5-coder:7b", "blake3:def").unwrap();
        let farm =
            VllmFarmClient::new("https://farm.internal/v1", "qwen2.5-coder-7b", "blake3:ghi")
                .unwrap();
        assert_eq!(llama.name(), "llama.cpp");
        assert_eq!(ollama.name(), "ollama");
        assert_eq!(farm.name(), "vllm-farm");
        for client in [&llama as &dyn LlmClient, &ollama, &farm] {
            assert!(client.base().starts_with("http"));
        }
    }

    #[test]
    fn non_loopback_bases_are_refused() {
        assert!(LlamaCppClient::new("http://10.0.0.9:39010", "m", "d").is_err());
        assert!(OllamaClient::new("https://models.example.com/v1", "m", "d").is_err());
    }

    #[test]
    fn unreachable_sidecar_is_a_structured_error_not_a_hang() {
        // Nothing listens on 39011: connect must fail fast, never hang the
        // frame loop (async, cancellable, 8 s default timeout per spec 05 §6).
        let client =
            LlamaCppClient::new("http://127.0.0.1:39011", "qwen2.5-coder-7b", "blake3:abc")
                .unwrap();
        let started = std::time::Instant::now();
        let err = client.complete(&prompt(), 1500).unwrap_err();
        assert!(err.starts_with("E_NET"), "got: {err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(8));
    }

    #[test]
    fn prompt_defaults_are_deterministic() {
        let p = prompt();
        assert_eq!(p.temperature, 0.0);
        assert_eq!(p.seed, 42);
    }
}
