# Phase 2: Agent Config System

## Priority: Critical | Effort: M | Status: completed

## Overview

Implement the config directory structure and Rust loading system for agent definitions. Supports hot-reload for prompt engineering iteration.

## Current Reality

- `config/agents/` contains 4 agent directories plus shared prompt assets.
- `src/agents/config.rs` loads TOML, shared includes, schema fragments, few-shot examples, and computes prompt hashes.
- `src/agents/hot_reload.rs` watches config files and reloads registry in-process.
- Hot reload logic exists, but no automated file-change test has been added yet.

## Requirements

- `config/agents/` directory with 4 agent configs + shared resources
- Rust struct to deserialize TOML configs
- Hot-reload via `notify` crate — prompt changes apply without restart
- Shared prompt inclusion (persona, risk levels)

## Architecture

```
config/agents/
├── summarizer/
│   ├── prompt.md
│   ├── config.toml
│   └── examples.json
├── url-assessor/
│   ├── prompt.md
│   ├── config.toml
│   └── examples.json
├── extractor/
│   ├── prompt.md
│   ├── config.toml
│   └── examples.json
├── detective/
│   ├── prompt.md
│   ├── config.toml
│   └── examples.json
└── shared/
    ├── persona.md
    ├── risk-levels.md
    └── output-schemas.json
```

### Config TOML Schema

```toml
[model]
endpoint = "http://localhost:8102/v1/chat/completions"
name = "qwen3.5-4b"
temperature = 0.3
max_tokens = 300
top_p = 0.9
enable_thinking = false   # Qwen3 thinking mode OFF — avoid reasoning token overhead

[response]
format = "json"           # Force JSON output mode
stream = false            # JSON agents: non-streaming (parse complete response)
schema = "shared/output-schemas.json#summarizer"

[prompt]
system = "prompt.md"
include_shared = ["persona.md"]
few_shot = "examples.json"

[runtime]
timeout_ms = 10000
retry_count = 1
```

## Implementation Steps

1. Create `src/agents/mod.rs` — module declarations
2. Create `src/agents/config.rs`:
   - `AgentConfig` struct (deserialized from TOML)
   - `AgentRegistry` struct: `Arc<RwLock<HashMap<String, AgentConfig>>>`
   - `load_all_agents(config_dir: &Path) -> AgentRegistry`
   - Loads each agent dir: reads config.toml, resolves prompt.md, includes shared files
3. Create `src/agents/prompt.rs`:
   - `build_system_prompt(agent: &AgentConfig) -> String`
   - Concatenates: shared includes → agent prompt.md
   - `build_messages(agent: &AgentConfig, user_content: &str) -> Vec<Message>`
   - Adds few-shot examples from examples.json
4. Create `src/agents/hot_reload.rs`:
   - Spawn background task: `notify::recommended_watcher` on `config/agents/`
   - On file change → reload affected agent config → update registry
5. Create all config files (prompt.md, config.toml, examples.json) for 4 agents + shared
6. Wire into `main.rs` — load registry at startup, pass as Axum state

## Related Files

- Create: `src/agents/mod.rs`, `src/agents/config.rs`, `src/agents/prompt.rs`, `src/agents/hot_reload.rs`
- Create: `config/agents/summarizer/*`, `config/agents/url-assessor/*`, `config/agents/extractor/*`, `config/agents/detective/*`, `config/agents/shared/*`

## Success Criteria

- [x] `AgentRegistry` loads all 4 agents without error
- [x] System prompt correctly includes shared files
- [x] Few-shot examples parsed and appended to messages
- [x] Hot-reload detects file change and updates config in-memory
- [ ] Unit test: modify prompt.md → verify registry reflects change

## Risk Assessment

- **notify crate race conditions** — Use debounce (500ms) to avoid partial reads during save
- **TOML parse errors in production** — Log error, keep old config, don't crash
