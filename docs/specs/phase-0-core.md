# Spec: Phase 0 — Core Harness (`huma-core` / `humad`)

## Goal

Deliver a headless Rust daemon (`humad`) managing >= 3 concurrent repository agent sessions over a local Unix Domain Socket (`NDJSON`), verified live against `agy acp --stdio`, with structured logging and append-only audit trail.

## Non-Goals

- No SQLite persistence (`huma-store` starts in Phase 2).
- No native macOS UI (`apps/macos` starts in Phase 1).
- No PTY terminal emulation.
- No remote SSH transport.

## Contract

- **Socket path**: `$HUMA_SOCKET_PATH`, or `$HUMA_STATE_DIR/humad.sock`, or `~/.local/state/huma/humad.sock` (mode `0600`).
- **Audit path**: `$HUMA_STATE_DIR/audit/audit.jsonl` or `~/.local/state/huma/audit/audit.jsonl` (mode `0600`, append-only JSONL, `v: 1`).
- **Wire protocol (`huma-protocol`)**:
  - Client -> Server: `ClientRequest` (`ping`, `list_projects`, `open_project`, `send_prompt`, `cancel_turn`, `respond_permission`, `close_project`, `subscribe`).
  - Server -> Client: `ServerEnvelope::Response` and `ServerEnvelope::Event` (`project_opened`, `project_closed`, `project_status_changed`, `semantic_title_updated`, `slash_commands_updated`, `activity_updated`, `message_delta`, `turn_completed`, `usage_updated`, `permission_requested`, `permission_resolved`).
- **Noise Folding**: `ToolActivity` updates collapse into a single `ActivityState` on the project snapshot; raw thought chunks are not emitted as primary message deltas.
- **Auto Semantic Title**: Defaults to repo basename on open; auto-updates from first user prompt summary and upgrades immediately when ACP emits `session_info_update.title`.
- **CLI Control (`humad ctl`)**: `ping`, `open`, `list`, `prompt`, `cancel`, `permit`, `close`, `watch`.

## Verification

1. `./scripts/check.sh` exits 0 (`arch_check.py`, `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`).
2. Unit tests cover: ACP wire parsing, multimodal block encoding, pure `DaemonState` transitions, auto-title & summary derivation, audit JSONL redaction, Unix Socket request/event roundtrip.
3. Live smoke test: start `humad serve`, open 3 concurrent repo sessions with `agy acp --stdio`, run prompts, disconnect & reconnect client confirming sessions stay live, inspect `audit.jsonl` confirming zero leaked payloads/secrets.
