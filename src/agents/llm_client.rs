use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::agents::config::{LoadedAgent, ResponseFormat};
use crate::agents::prompt::Message;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub content: String,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: Client,
}

impl LlmClient {
    pub fn new() -> AppResult<Self> {
        let http = Client::builder()
            .pool_idle_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http })
    }

    pub async fn complete_json<T: DeserializeOwned>(
        &self,
        agent: &LoadedAgent,
        user_content: &str,
    ) -> AppResult<T> {
        let messages = build_messages(agent, user_content);
        let first = self
            .complete_with_messages(agent, messages.clone(), false)
            .await?;
        if let Some(value) = repair_json::<T>(&first.content)? {
            return Ok(value);
        }

        let mut retry_messages = messages;
        retry_messages.push(Message {
            role: "user".to_string(),
            content: "Output JSON only. No explanation.".to_string(),
        });
        let second = self
            .complete_with_messages(agent, retry_messages, false)
            .await?;
        repair_json::<T>(&second.content)?.ok_or_else(|| {
            tracing::warn!(
                agent = %agent.key,
                first_preview = %preview_text(&first.content),
                second_preview = %preview_text(&second.content),
                "model returned invalid JSON after retry"
            );
            AppError::Server("invalid JSON response from model".to_string())
        })
    }

    pub async fn stream_text(
        &self,
        agent: &LoadedAgent,
        user_content: &str,
        tx: mpsc::Sender<StreamChunk>,
    ) -> AppResult<String> {
        if !agent.response.stream {
            return Err(AppError::Config(format!(
                "agent {} is not configured for streaming",
                agent.key
            )));
        }
        let body = request_body(agent, build_messages(agent, user_content), true);
        let response = self.send_with_retry(agent, &body).await?;
        let mut final_text = String::new();
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();
                if !line.starts_with("data: ") {
                    continue;
                }
                let data = &line[6..];
                if data == "[DONE]" {
                    let _ = tx
                        .send(StreamChunk {
                            content: String::new(),
                            done: true,
                        })
                        .await;
                    return Ok(final_text);
                }
                let value: Value = serde_json::from_str(data)
                    .map_err(|error| AppError::Server(format!("invalid SSE payload: {error}")))?;
                if let Some(content) = value["choices"][0]["delta"]["content"].as_str() {
                    final_text.push_str(content);
                    let _ = tx
                        .send(StreamChunk {
                            content: content.to_string(),
                            done: false,
                        })
                        .await;
                }
            }
        }

        let _ = tx
            .send(StreamChunk {
                content: String::new(),
                done: true,
            })
            .await;
        Ok(final_text)
    }

    async fn complete_with_messages(
        &self,
        agent: &LoadedAgent,
        messages: Vec<Message>,
        stream: bool,
    ) -> AppResult<CompletionResponse> {
        let body = request_body(agent, messages, stream);
        let response: Value = self.send_with_retry(agent, &body).await?.json().await?;

        let message = &response["choices"][0]["message"];
        let content = message["content"]
            .as_str()
            .or_else(|| message["reasoning"].as_str())
            .unwrap_or_default()
            .to_string();
        let _ = response.get("usage");
        Ok(CompletionResponse { content })
    }

    async fn send_with_retry(
        &self,
        agent: &LoadedAgent,
        body: &Value,
    ) -> AppResult<reqwest::Response> {
        let max_attempts = usize::from(agent.runtime.retry_count) + 1;
        for attempt in 1..=max_attempts {
            let response = match self
                .http
                .post(&agent.model.endpoint)
                .timeout(Duration::from_millis(agent.runtime.timeout_ms))
                .json(body)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let retryable = error.is_timeout() || error.is_connect() || error.is_request();
                    if retryable && attempt < max_attempts {
                        sleep(Duration::from_secs(attempt as u64)).await;
                        continue;
                    }
                    return Err(AppError::Http(error));
                }
            };

            if response.status().is_success() {
                return Ok(response);
            }

            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            let retryable = status.as_u16() == 429 || status.is_server_error();
            if retryable && attempt < max_attempts {
                sleep(Duration::from_secs(attempt as u64)).await;
                continue;
            }

            return Err(AppError::Server(format!(
                "llm upstream error {} at {}: {}",
                status, agent.model.endpoint, error_body
            )));
        }

        Err(AppError::Server(format!(
            "llm request exhausted retries for {}",
            agent.model.endpoint
        )))
    }
}

fn build_messages(agent: &LoadedAgent, user_content: &str) -> Vec<Message> {
    let mut messages = vec![Message {
        role: "system".to_string(),
        content: agent.system_prompt.clone(),
    }];
    messages.extend(agent.few_shot.clone());
    messages.push(Message {
        role: "user".to_string(),
        content: user_content.to_string(),
    });
    messages
}

fn request_body(agent: &LoadedAgent, mut messages: Vec<Message>, stream: bool) -> Value {
    let use_content_prefill = should_use_content_prefill(agent, stream);
    if use_content_prefill {
        messages.push(Message {
            role: "assistant".to_string(),
            content: String::new(),
        });
    }

    let mut body = json!({
        "model": agent.model.name,
        "messages": messages,
        "temperature": agent.model.temperature,
        "max_tokens": agent.model.max_tokens,
        "top_p": agent.model.top_p,
        "stream": stream,
        "chat_template_kwargs": {
            "enable_thinking": agent.model.enable_thinking
        }
    });

    if use_content_prefill {
        // llama.cpp on the local 8001 JSON worker still opens `<think>` even when
        // enable_thinking=false. Assistant prefill keeps generation on content.
        body["continue_final_message"] = json!(true);
        body["add_generation_prompt"] = json!(false);
    }

    if agent.response.format == ResponseFormat::Json {
        body["response_format"] = json!({ "type": "json_object" });
    }

    body
}

fn should_use_content_prefill(agent: &LoadedAgent, stream: bool) -> bool {
    !stream
        && agent.response.format == ResponseFormat::Json
        && matches_local_reasoning_endpoint(&agent.model.endpoint)
}

fn matches_local_reasoning_endpoint(endpoint: &str) -> bool {
    endpoint.contains("://localhost:8001/")
        || endpoint.contains("://127.0.0.1:8001/")
        || endpoint.contains("://host.docker.internal:8001/")
}

fn repair_json<T: DeserializeOwned>(content: &str) -> AppResult<Option<T>> {
    let content = content.trim();
    if let Ok(value) = serde_json::from_str::<T>(content) {
        return Ok(Some(value));
    }

    if let Some(stripped) = strip_code_fence(content) {
        if let Ok(value) = serde_json::from_str::<T>(stripped) {
            return Ok(Some(value));
        }
    }

    // Strip thinking tokens: <think>...</think>
    let content = strip_thinking_tags(content);
    let content = content.trim();

    let start = content.find(['{', '[']);
    let end = content.rfind(['}', ']']);
    let Some((start, end)) = start.zip(end) else {
        return Ok(None);
    };
    let candidate = &content[start..=end];
    if let Ok(value) = serde_json::from_str::<T>(candidate) {
        return Ok(Some(value));
    }

    // Try fixing truncated JSON by closing open braces/brackets
    if let Some(value) = try_close_json::<T>(candidate) {
        return Ok(Some(value));
    }

    // Try removing trailing comma before closing brace/bracket
    let fixed = candidate.replace(",}", "}").replace(",]", "]");
    if let Ok(value) = serde_json::from_str::<T>(&fixed) {
        return Ok(Some(value));
    }

    Ok(None)
}

fn strip_thinking_tags(content: &str) -> &str {
    if let Some(end) = content.find("</think>") {
        content[end + 8..].trim()
    } else {
        content
    }
}

fn try_close_json<T: DeserializeOwned>(content: &str) -> Option<T> {
    let mut open_braces = 0i32;
    let mut open_brackets = 0i32;
    let mut in_string = false;
    let mut escape_next = false;

    for ch in content.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => open_braces += 1,
            '}' if !in_string => open_braces -= 1,
            '[' if !in_string => open_brackets += 1,
            ']' if !in_string => open_brackets -= 1,
            _ => {}
        }
    }

    if open_braces <= 0 && open_brackets <= 0 {
        return None;
    }

    let mut fixed = content.to_string();
    // Remove potential trailing incomplete key-value
    if let Some(last_comma) = fixed.rfind(',') {
        let after = fixed[last_comma + 1..].trim();
        if after.starts_with('"') && !after.contains(':') {
            fixed.truncate(last_comma);
        }
    }
    for _ in 0..open_brackets {
        fixed.push(']');
    }
    for _ in 0..open_braces {
        fixed.push('}');
    }
    serde_json::from_str::<T>(&fixed).ok()
}

fn strip_code_fence(content: &str) -> Option<&str> {
    let content = content.trim();
    let content = content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```"))?;
    content.strip_suffix("```").map(str::trim)
}

fn preview_text(content: &str) -> String {
    let collapsed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let preview = chars.by_ref().take(240).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}
