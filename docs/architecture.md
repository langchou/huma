# Huma Architecture

## Layers

```text
apps/macos (Swift 6: SwiftUI + AppKit)
  │ Unix Domain Socket (NDJSON, JSON-RPC-like ClientRequest / ServerEnvelope)
  ▼
humad (Rust daemon)
  ├── huma-acp      (ACP stdio JSON-RPC 2.0 + Stream-JSON drivers)
  ├── huma-store    (SQLite WAL: projects, auto titles, latest summaries, session IDs)
  └── huma-protocol (pure wire types, UI-neutral, backward-compatible serde)
```

## Transport Boundary (`humad` <-> UI)

- Local Unix Domain Socket (`~/.local/state/huma/humad.sock` or `$HUMA_SOCKET_PATH`).
- Framing: newline-delimited JSON (`ClientRequest` in; `ServerEnvelope::Response` | `ServerEnvelope::Event` out).
- UI owns presentation, pacing, animations, and window lifecycle; `humad` owns child processes, protocol state, noise folding, and persistence.
- Closing or restarting UI MUST NOT terminate running agent processes.

## Driver Contract (`huma-acp`)

Unified `HarnessEvent` stream produced by all drivers:
- `MessageChunk(String)` — raw assistant text chunk
- `ThoughtChunk(String)` — reasoning text (kept out of primary view by default)
- `ToolActivity { title, status, detail }` — folded into single-line `ActivityState`
- `SlashCommands(Vec<SlashCommand>)` — live command autocomplete source
- `TitleUpdated(String)` — semantic project title update
- `UsageUpdated` / `PermissionRequested` / `TurnFinished` / `TurnFailed` / `ProcessExited`

Initial local driver invocations (BYOS):
- `Agy`: `agy acp --stdio` (supports `image: true`, `session/load`, `available_commands_update`, `session_info_update`)
- `Opencode`: `opencode acp --cwd <cwd>`
- `ClaudeCode`: `claude --print --input-format=stream-json --output-format=stream-json` (planned native stream driver)
- `CustomAcp`: user-supplied argv implementing ACP over stdio

## Client Streaming Pipeline (`apps/macos/Sources/HumaRenderer`)

1. Receive `ServerEvent::MessageDelta` from `humad` without blocking.
2. Buffer text in adaptive `TokenPacer` driven by 120Hz display link; accelerate smoothly if backlog grows.
3. Render incremental Markdown AST with soft opacity fade-in (60–80ms) and smooth height interpolation.
4. Render `ServerEvent::ActivityUpdated` exclusively in single-line collapsible header capsule.

## Repo Layout

```text
huma/
├── AGENTS.md / CLAUDE.md
├── justfile
├── scripts/        # check.sh, arch_check.py
├── docs/           # product.md, architecture.md, roadmap.md, harness.md, specs/
├── core/           # Cargo workspace: huma-protocol, huma-acp, huma-store, humad
└── apps/macos/     # SwiftPM package: HumaClient, HumaRenderer, HumaApp
```
