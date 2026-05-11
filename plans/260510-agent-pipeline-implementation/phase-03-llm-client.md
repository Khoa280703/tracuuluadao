# Phase 3: LLM Client

## Priority: Critical | Effort: M | Status: completed

## Overview

HTTP client for calling vLLM OpenAI-compatible API. Supports both streaming and non-streaming responses. JSON mode for structured output.

## Current Reality

- `src/agents/llm_client.rs` implements non-streaming completions, JSON parsing/repair, SSE chunk parsing, per-agent timeout, and retry behavior.
- The live harness now validates real upstream behavior for direct 4B JSON, direct 27B streaming, and 4 parallel 4B requests.
- Validation evidence is recorded in `plans/reports/validation-260511-203629-agent-pipeline-live.md`.

## Requirements

- Call vLLM `/v1/chat/completions` endpoint
- Streaming SSE response parsing (token-by-token)
- Non-streaming JSON response for structured agents
- Timeout handling, retry logic
- Connection pooling for concurrent requests

## Architecture

```rust
// Core trait
pub struct LlmClient {
    http: reqwest::Client,    // connection pool
}

impl LlmClient {
    pub async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse>
    pub fn stream(&self, req: CompletionRequest) -> impl Stream<Item = Result<StreamChunk>>
}
```

### vLLM OpenAI-compatible API

```
POST http://localhost:8102/v1/chat/completions
{
  "model": "qwen3.5-4b",
  "messages": [{"role": "system", "content": "..."}, {"role": "user", "content": "..."}],
  "temperature": 0.3,
  "max_tokens": 300,
  "stream": false,                              // JSON agents: non-streaming
  "response_format": {"type": "json_object"},   // force JSON output
  "chat_template_kwargs": {
    "enable_thinking": false                    // CRITICAL: disable Qwen3 thinking mode
  }
}

Non-stream response: {"choices": [{"message": {"content": "..."}}]}
Stream response (Detective only): SSE with data: {"choices": [{"delta": {"content": "..."}}]}
```

### JSON Repair Mechanism (mandatory for all JSON agents)

```
1. Parse response.content as JSON
2. If parse fails → regex extract first {...} or [...] from content
3. If still fails → retry LLM call with appended message: "Output JSON only. No explanation."
4. If 2nd attempt fails → return error (no infinite loop)
```

## Implementation Steps

1. Create `src/agents/llm_client.rs`:
   - `LlmClient::new()` — create reqwest client with connection pool
   - `CompletionRequest` struct: messages, model config (from AgentConfig)
   - `CompletionResponse` struct: content string, usage stats
   - `StreamChunk` struct: delta content, finish_reason
2. Implement `complete()`:
   - Build request body from AgentConfig + user content
   - POST to endpoint, parse JSON response
   - Handle errors: timeout, 429 rate limit, 500 server error
3. Implement `stream()`:
   - Same request with `"stream": true`
   - Parse SSE lines: `data: {...}` → extract delta content
   - Handle `data: [DONE]` termination
   - Return `tokio_stream::Stream` of chunks
4. Add retry logic:
   - On timeout/5xx: retry up to `retry_count` from config
   - On 429: exponential backoff (1s, 2s)
   - On parse error for JSON agents: run JSON repair flow exactly once, then return error if still invalid
5. Create helper: `call_agent(registry, agent_name, user_content) -> Result<String>`
   - Convenience function: load config → build messages → call complete
6. Create helper: `stream_agent(registry, agent_name, user_content) -> impl Stream`
   - Same but streaming

## Related Files

- Create: `src/agents/llm_client.rs`
- Modify: `src/agents/mod.rs` (add module)

## Key Code Snippet

```rust
pub fn stream(&self, req: CompletionRequest) -> impl Stream<Item = Result<StreamChunk>> {
    let http = self.http.clone();
    async_stream::stream! {
        let resp = http.post(&req.endpoint)
            .json(&req.to_body(true))
            .send().await?;
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();
                if line.starts_with("data: ") {
                    let data = &line[6..];
                    if data == "[DONE]" { return; }
                    if let Ok(obj) = serde_json::from_str::<SseData>(data) {
                        if let Some(content) = obj.choices[0].delta.content.as_ref() {
                            yield Ok(StreamChunk { content: content.clone(), done: false });
                        }
                    }
                }
            }
        }
    }
}
```

## Success Criteria

- [x] Non-streaming and streaming client code paths are implemented
- [x] JSON repair flow exists for structured agents
- [x] Non-streaming call verified against live 4B model in this pass
- [x] Streaming call verified against live upstream in this pass
- [x] Timeout triggers retry for transport-level timeout/errors
- [x] Concurrent calls (4 parallel) verified against live upstream

## Risk Assessment

- **vLLM SSE format differences** — Test actual vLLM output format, may differ slightly from OpenAI
- **JSON mode enforcement** — Some models ignore `response_format`, may need prompt-level enforcement ("You MUST respond in JSON")
