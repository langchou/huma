# Logging & Audit Standard

## Scope

Applies to all runtime diagnostics and immutable audit records across `core/` and `apps/macos/`.

## Runtime Logging Rules

- **Rust (`core/`)**: MUST use `tracing` (`error!`, `warn!`, `info!`, `debug!`, `trace!`). MUST NOT use `println!`, `eprintln!`, or `dbg!` in library crates or daemon runtime (only explicit CLI stdout handlers may write stdout).
- **Swift (`apps/macos/`)**: MUST use `os.Logger` (`subsystem: "dev.huma.macos"`, categories: `client`, `renderer`, `gestures`, `lifecycle`). MUST NOT use `print()` or `NSLog()`.
- **Level Semantics**:
  - `ERROR`: unrecoverable failure, driver crash, protocol corruption, storage failure.
  - `WARN`: recoverable anomaly, dropped unknown update, pacer backlog pressure, stale socket cleanup.
  - `INFO`: lifecycle boundaries only (daemon start/stop, project open/close, driver spawn/exit, turn start/end, permission request/decision).
  - `DEBUG`: RPC method names, state transitions, durations (`elapsed_ms`).
  - `TRACE`: raw wire frames (opt-in via `HUMA_LOG=trace` only; NEVER default).
- **Correlation Fields**: Session/turn logs MUST include `project_id`, `driver`, and when present `session_id`, `turn_id`.
- **Storage & Rotation**: `~/.local/state/huma/logs/` (`humad.log`, `huma-macos.log`), structured JSONL or key=value, max `10MB x 5` files, file mode `0600`.

## Audit Trail Rules

- **Path**: Append-only JSONL at `~/.local/state/huma/audit/audit.jsonl` (file mode `0600`).
- **Envelope**: `{"v":1,"seq":<u64>,"ts_ms":<u64>,"event":"<name>","project_id":"...","data":{...}}`.
- **Mandatory Audited Events**:
  - `project.opened` / `project.closed` (`cwd`, `driver`)
  - `driver.spawned` / `driver.exited` (`driver`, redacted `argv`, `exit_code`)
  - `prompt.submitted` (`turn_id`, `text_len`, `attachment_count`)
  - `permission.requested` / `permission.resolved` (`request_id`, `tool_title`, `decision`, `option_id`)
  - `turn.completed` / `turn.failed` / `turn.cancelled` (`turn_id`, `stop_reason`, `usage`, `elapsed_ms`)
- **Failure Policy**: Audit write errors MUST emit `WARN` and MUST NOT crash live sessions.

## Privacy & Redaction (Hard Rule)

- MUST NEVER write secrets (`*_KEY`, `*_TOKEN`, `*_SECRET`, `Authorization`, cookies) to logs or audit files.
- MUST NEVER log raw base64 image/file payloads; log `mime_type` and `byte_len` only.
- MUST NOT log raw prompt text or full assistant output in default `INFO`/`WARN`/`ERROR` logs or audit records (payload logging requires explicit `HUMA_LOG_PAYLOADS=1`).

## Mechanical Gates

- `scripts/arch_check.py` forbids `println!`/`eprintln!`/`dbg!` in `huma-protocol`, `huma-acp`, `huma-store`, and `print(`/`NSLog(` in `apps/macos/Sources`.
