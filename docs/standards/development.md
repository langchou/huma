# Development Standard

Pure agent-authored codebase. No manual code edits.

## Scope

Applies to `core/` (Rust), `apps/macos/` (Swift), and `scripts/`.

## Workflow Rules

- **Worktree Isolation**: `main` is the stable branch. MUST NOT develop directly on `main`. For every task:
  1. Branch a worktree from `main`: `git worktree add ../huma-worktrees/<slug> -b <type>/<slug> main`.
  2. Implement and run all tests inside that worktree (`./scripts/check.sh` + task verification).
  3. Fast-forward/merge into `main` only after all checks pass, push `origin/main`, then clean up (`git worktree remove` + `git branch -d`).
- **Spec First**: Non-trivial changes MUST have an approved `docs/specs/<name>.md` before coding.
- **Surgical Diff**: Modify only files required by the task. No unrelated refactors.
- **Dependency Freeze**: MUST NOT add entries to `Cargo.toml` or `Package.swift` without explicit human approval.
- **Self-Healing Gate**: MUST run `./scripts/check.sh` before finishing. Fix root causes up to 3 attempts; escalate with facts if blocked. MUST NOT weaken assertions or silence linters to pass.
- **Git & Identity**: Repo author MUST be `langchou <71913459+langchou@users.noreply.github.com>`. Lowercase conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`), no emojis, no `Co-authored-by` trailers (enforced by `.githooks/commit-msg`).

## Architecture & Module DAG

```text
huma-protocol ◀── huma-acp   ──┐
      ▲                        ├──▶ humad (Unix Socket NDJSON) ◀── apps/macos (Swift)
      └────────── huma-store ──┘                               ◀── future native UIs
```

- `huma-protocol`: pure data + wire types only; zero `tokio`, zero I/O, zero UI terms (`sidebar`, `capsule`, `window`, `theme`, `color`, `animation`). New JSON fields MUST use `#[serde(default)]`; open enums MUST carry `#[serde(other)] Unknown`.
- `huma-acp` / `huma-store`: MUST depend only on `huma-protocol`.
- `apps/macos`: MUST NOT spawn `Process` directly; all agent execution flows through `humad`.
- Pure logic (parsers, reducers, pacers) MUST be unit-testable without live agents, sockets, or windows.

## Language Invariants

- **File Ceiling**: <= 500 lines per production `.rs` (excluding `#[cfg(test)]`) and `.swift` file.
- **Rust**: MUST pass `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`. MUST NOT use `.unwrap()`, `.expect()`, `todo!()`, `unimplemented!()`, `dbg!()` in prod. Every `#[allow(...)]` MUST have an adjacent comment. Use `thiserror` + `?`.
- **Swift**: Swift 6 strict concurrency (`async`/`await`, `actor`, `@MainActor`). MUST NOT use `try!`, force-unwrap `!`, or unjustified `fatalError` / `DispatchQueue`.
- **UI Craft**: 120Hz interruptible spring transitions (180–240ms); streamed text MUST pass through Smooth Ink Pacer; collapse tool noise into one line by default.

## Mechanical Gates

- `./scripts/check.sh` runs `scripts/arch_check.py`, Rust fmt/clippy/test, and Swift test.
- Milestone sweep: delete dead code, split files approaching 400 lines, promote repeated review findings into `scripts/arch_check.py`.
