use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FewShotFile {
    Messages(Vec<Message>),
    Wrapped { messages: Vec<Message> },
    Examples(Vec<FewShotExample>),
    WrappedExamples { examples: Vec<FewShotExample> },
}

#[derive(Debug, Deserialize)]
struct FewShotExample {
    input: String,
    output: Value,
}

pub fn read_text(path: &Path) -> AppResult<String> {
    Ok(fs::read_to_string(path)?)
}

pub fn read_schema_fragment(shared_dir: &Path, schema_ref: &str) -> AppResult<String> {
    let (file_name, key) = schema_ref.split_once('#').unwrap_or((schema_ref, ""));
    let raw = fs::read_to_string(shared_dir.join(file_name))?;
    if key.is_empty() {
        return Ok(raw);
    }

    let root: Value = serde_json::from_str(&raw)
        .map_err(|error| AppError::Server(format!("invalid schema JSON {file_name}: {error}")))?;
    let fragment = root
        .get(key)
        .cloned()
        .ok_or_else(|| AppError::Config(format!("missing schema fragment: {schema_ref}")))?;
    serde_json::to_string_pretty(&fragment)
        .map_err(|error| AppError::Server(format!("failed to render schema {schema_ref}: {error}")))
}

pub fn load_few_shot_messages(path: &Path) -> AppResult<Vec<Message>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(path)?;
    let parsed: FewShotFile = serde_json::from_str(&raw).map_err(|error| {
        AppError::Server(format!("invalid few-shot JSON {}: {error}", path.display()))
    })?;
    Ok(match parsed {
        FewShotFile::Messages(messages) => messages,
        FewShotFile::Wrapped { messages } => messages,
        FewShotFile::Examples(examples) | FewShotFile::WrappedExamples { examples } => {
            examples_to_messages(examples)
        }
    })
}

fn examples_to_messages(examples: Vec<FewShotExample>) -> Vec<Message> {
    let mut messages = Vec::with_capacity(examples.len() * 2);
    for example in examples {
        messages.push(Message {
            role: "user".to_string(),
            content: example.input,
        });
        messages.push(Message {
            role: "assistant".to_string(),
            content: render_output(example.output),
        });
    }
    messages
}

fn render_output(output: Value) -> String {
    match output {
        Value::String(text) => text,
        Value::Object(map) if map.len() == 1 => map
            .get("report_pattern")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| Value::Object(map).to_string()),
        other => other.to_string(),
    }
}
