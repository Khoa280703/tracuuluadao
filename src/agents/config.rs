use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::agents::prompt::{Message, load_few_shot_messages, read_schema_fragment, read_text};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub endpoint: String,
    pub name: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub top_p: f32,
    #[serde(default)]
    pub enable_thinking: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResponseFormat {
    Json,
    Markdown,
    Text,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseConfig {
    pub format: ResponseFormat,
    #[serde(default)]
    pub stream: bool,
    pub schema: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptConfig {
    pub system: String,
    #[serde(default)]
    pub include_shared: Vec<String>,
    pub few_shot: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
    pub timeout_ms: u64,
    pub retry_count: u8,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentFileConfig {
    pub model: ModelConfig,
    pub response: ResponseConfig,
    pub prompt: PromptConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone)]
pub struct LoadedAgent {
    pub key: String,
    pub model: ModelConfig,
    pub response: ResponseConfig,
    pub runtime: RuntimeConfig,
    pub system_prompt: String,
    pub few_shot: Vec<Message>,
    pub prompt_hash: String,
}

#[derive(Debug)]
pub struct AgentRegistry {
    config_dir: PathBuf,
    agents: RwLock<HashMap<String, Arc<LoadedAgent>>>,
}

impl AgentRegistry {
    pub fn load_all(config_dir: impl AsRef<Path>) -> AppResult<Arc<Self>> {
        let config_dir = config_dir.as_ref().to_path_buf();
        let agents = load_agents(&config_dir)?;
        Ok(Arc::new(Self {
            config_dir,
            agents: RwLock::new(agents),
        }))
    }

    pub fn get(&self, key: &str) -> AppResult<Arc<LoadedAgent>> {
        self.agents
            .read()
            .map_err(|_| AppError::Server("agent registry lock poisoned".to_string()))?
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::Server(format!("agent not found: {key}")))
    }

    pub fn reload(&self) -> AppResult<()> {
        let agents = load_agents(&self.config_dir)?;
        *self
            .agents
            .write()
            .map_err(|_| AppError::Server("agent registry lock poisoned".to_string()))? = agents;
        Ok(())
    }

    pub fn prompt_hash_all(&self) -> AppResult<String> {
        let agents = self
            .agents
            .read()
            .map_err(|_| AppError::Server("agent registry lock poisoned".to_string()))?;
        let mut hashes = agents
            .values()
            .map(|agent| agent.prompt_hash.clone())
            .collect::<Vec<_>>();
        hashes.sort();
        Ok(hash_text(&hashes.join("|")))
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }
}

fn load_agents(config_dir: &Path) -> AppResult<HashMap<String, Arc<LoadedAgent>>> {
    let shared_dir = config_dir.join("shared");
    let mut agents = HashMap::new();

    for entry in fs::read_dir(config_dir)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("shared") {
            continue;
        }

        let key = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::Config("invalid agent directory".to_string()))?
            .to_owned();
        let raw_config = fs::read_to_string(path.join("config.toml"))?;
        let config: AgentFileConfig = toml::from_str(&raw_config).map_err(|error| {
            AppError::Config(format!("invalid agent config for {key}: {error}"))
        })?;
        let model = config.model;
        let mut system_parts = Vec::new();
        for include in &config.prompt.include_shared {
            system_parts.push(read_text(&shared_dir.join(include))?);
        }
        system_parts.push(read_text(&path.join(&config.prompt.system))?);
        if let Some(schema_ref) = config.response.schema.as_deref() {
            system_parts.push(format!(
                "## Output schema\n{}",
                read_schema_fragment(&shared_dir, schema_ref)?
            ));
        }
        let system_prompt = system_parts.join("\n\n");
        let few_shot = config
            .prompt
            .few_shot
            .as_ref()
            .map(|file| load_few_shot_messages(&path.join(file)))
            .transpose()?
            .unwrap_or_default();
        let prompt_hash_material = format!(
            "{raw_config}\n{system_prompt}\n{}",
            serde_json::to_string(&few_shot)?
        );

        agents.insert(
            key.clone(),
            Arc::new(LoadedAgent {
                key,
                model,
                response: config.response,
                runtime: config.runtime,
                prompt_hash: hash_text(&prompt_hash_material),
                system_prompt,
                few_shot,
            }),
        );
    }

    Ok(agents)
}

fn hash_text(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}
