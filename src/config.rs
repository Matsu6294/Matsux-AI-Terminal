//! Persistent configuration for matsux-term.
//! Saved to ~/.matsux-term.toml

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─── Config struct ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub last_repo_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            model: std::env::var("MATSUX_MODEL")
                .or_else(|_| std::env::var("OPENAI_MODEL"))
                .unwrap_or_else(|_| "gpt-4o".to_string()),
            last_repo_path: None,
        }
    }
}

// ─── Persistence ─────────────────────────────────────────────────────────────

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".matsux-term.toml")
}

pub fn load() -> Config {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let content = toml::to_string_pretty(cfg).unwrap_or_default();
    std::fs::write(config_path(), content)
}

// ─── Provider presets ─────────────────────────────────────────────────────────

pub struct Provider {
    pub name: &'static str,
    pub base_url: &'static str,
    pub models: &'static [&'static str],
    /// Placeholder shown when API key not yet set.
    pub key_hint: &'static str,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        models: &["gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "o1-mini", "o3-mini"],
        key_hint: "sk-...",
    },
    Provider {
        name: "Groq (gratis)",
        base_url: "https://api.groq.com/openai/v1",
        models: &[
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "mixtral-8x7b-32768",
            "gemma2-9b-it",
        ],
        key_hint: "gsk_...",
    },
    Provider {
        name: "Mistral AI",
        base_url: "https://api.mistral.ai/v1",
        models: &["codestral-latest", "mistral-large-latest", "mistral-small-latest"],
        key_hint: "...",
    },
    Provider {
        name: "Ollama (lokal)",
        base_url: "http://localhost:11434/v1",
        models: &[
            "qwen2.5:1.5b",
            "qwen2.5-coder:1.5b",
            "qwen2.5-coder:3b",
            "qwen2.5-coder:7b",
            "llama3.2:3b",
            "phi3:mini",
            "deepseek-coder:1.3b",
        ],
        key_hint: "ollama",
    },
    Provider {
        name: "Anpassad",
        base_url: "",
        models: &[],
        key_hint: "...",
    },
];
