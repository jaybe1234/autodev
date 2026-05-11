use std::collections::HashMap;
use std::path::PathBuf;

use eyre::WrapErr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub jira: JiraConfig,
    pub github: GitHubConfig,
    pub opencode: OpencodeConfig,
    pub mapping: Vec<MappingEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    pub webhook_secret: String,
    #[serde(default = "default_storage_path")]
    pub storage_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JiraConfig {
    pub base_url: String,
    pub pat: String,
    #[serde(default = "default_transition_to")]
    pub transition_to: String,
    pub ready_to_dev_status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubConfig {
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpencodeConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MappingEntry {
    pub label: String,
    pub repo: String,
}

impl AppConfig {
    pub fn load(path: &std::path::Path) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading config from {}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("parsing config from {}", path.display()))?;
        Ok(config)
    }

    pub fn find_repo_for_label(&self, label: &str) -> Option<&str> {
        self.mapping
            .iter()
            .find(|m| m.label.eq_ignore_ascii_case(label))
            .map(|m| m.repo.as_str())
    }
}

fn default_bind() -> String {
    "0.0.0.0:3000".into()
}

fn default_storage_path() -> PathBuf {
    PathBuf::from("./data")
}

fn default_transition_to() -> String {
    "In Review".into()
}

fn default_model() -> String {
    "anthropic/claude-sonnet-4-20250514".into()
}
