# Agents File Ownership Micro-Plan

Context: bám `plans/260510-agent-pipeline-implementation`, chỉ scope `src/agents/*` và `config/agents/*`, phục vụ tích hợp Phase 5/6.

## 1. Thứ tự file nên tạo/sửa

1. `config/agents/shared/persona.md`, `risk-levels.md`, `output-schemas.json`
2. `config/agents/{summarizer,url-assessor,extractor,detective}/config.toml`
3. `config/agents/{summarizer,url-assessor,extractor,detective}/prompt.md`
4. `config/agents/{summarizer,url-assessor,extractor,detective}/examples.json`
5. `src/agents/config.rs`
6. `src/agents/prompt.rs`
7. `src/agents/llm_client.rs`
8. `src/agents/hot_reload.rs`
9. `src/agents/mod.rs`

Reason: chốt filesystem contract trước, rồi loader/prompt builder, rồi LLM transport, rồi watcher. `mod.rs` sửa cuối để export surface ổn định cho pipeline.

## 2. Public interface cần có cho Phase 5/6

```rust
pub struct AgentRegistry;
pub struct ResolvedAgentConfig;
pub struct ChatMessage;
pub struct LlmClient;

pub fn load_agent_registry(config_dir: &Path) -> AppResult<Arc<AgentRegistry>>;
pub fn spawn_agent_hot_reload(config_dir: PathBuf, registry: Arc<AgentRegistry>) -> AppResult<()>;

impl AgentRegistry {
    pub fn get(&self, agent_name: &str) -> AppResult<ResolvedAgentConfig>;
}

pub fn build_messages(agent: &ResolvedAgentConfig, user_input: &str) -> AppResult<Vec<ChatMessage>>;

impl LlmClient {
    pub async fn invoke_json(
        &self,
        agent: &ResolvedAgentConfig,
        messages: Vec<ChatMessage>,
    ) -> AppResult<String>;

    pub async fn stream_text(
        &self,
        agent: &ResolvedAgentConfig,
        messages: Vec<ChatMessage>,
    ) -> AppResult<impl Stream<Item = AppResult<String>>>;
}
```

Mapping:
- Phase 5 dùng `get` + `build_messages` + `invoke_json` cho `summarizer`, `url-assessor`, `extractor`
- Phase 5/6 dùng `get` + `build_messages` + `stream_text` cho `detective`
- Phase 6 không nên đọc file config trực tiếp; chỉ đi qua `AgentRegistry` và `LlmClient`

## 3. Rủi ro compile

- Tính snapshot `2026-05-10`, repo **đã có** `Cargo.toml` và `src/main.rs`, nên rủi ro “chưa có Cargo/main” hiện không áp dụng.
- Compile blocker thực tế hiện tại: `cargo check` fail ở native build của `boring-sys2` từ `rquest`, lỗi `fatal error: 'stddef.h' file not found`, nên có thể chưa verify được code Rust mới dù API đúng.
- Nếu export public API quá sớm ở `src/agents/mod.rs` nhưng type trong `config.rs`/`llm_client.rs` còn đổi tên, Phase 5/6 sẽ vỡ import ngay.
- Nếu `config/agents/*` chưa tồn tại nhưng `AppConfig.agent_config_dir` đã trỏ `./config/agents`, app sẽ fail từ startup path/load thay vì fail ở runtime pipeline.

## Unresolved questions

- Có muốn cố định tên public type là `ResolvedAgentConfig` hay giữ `AgentConfig` để khớp phase-02 plan?
- Có chấp nhận `stream_text` trả boxed stream để giữ signature đơn giản hơn cho Axum/SSE không?
