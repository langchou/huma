# Huma Agent Rules

Pure agent-authored repository. No manual code edits.

## Standards (Read First — Mandatory)

1. [docs/standards/documentation.md](docs/standards/documentation.md) — agent-native doc writing rules & line caps
2. [docs/standards/development.md](docs/standards/development.md) — workflow, module DAG, Rust/Swift & craft invariants
3. [docs/standards/logging-audit.md](docs/standards/logging-audit.md) — structured logging, audit JSONL & redaction rules

## System Truth

- [docs/product.md](docs/product.md) — product invariants, UX loops, non-goals
- [docs/architecture.md](docs/architecture.md) — layers, IPC transport, driver contract, streaming pipeline
- [docs/roadmap.md](docs/roadmap.md) — phase scope and exit criteria
- `docs/specs/<feature>.md` — approved feature specs (required before non-trivial implementation)

## Primary Gate

Every task MUST pass before reporting completion:

```bash
./scripts/check.sh        # arch + rust + swift (full gate)
./scripts/check.sh arch   # standards, layering, file/doc caps, anti-cheat
./scripts/check.sh rust   # fmt + clippy -D warnings + test
./scripts/check.sh swift  # swift test
```

## Non-Negotiable Summary

- Worktree per task from `main` (`../huma-worktrees/<slug>`); test in worktree before merging to `main`.
- Spec first (`docs/specs/`); surgical diffs only; dependency freeze without human approval.
- Strict DAG: `huma-protocol` <- `huma-acp` / `huma-store` <- `humad` <- `apps/macos`.
- Caps: prod `.rs`/`.swift` <= 500 lines; `AGENTS.md` <= 100 lines; every `docs/**/*.md` <= 100 lines.
- Zero `.unwrap()`, `.expect()`, `todo!()`, `dbg!()`, `try!`, unjustified `#[allow]`/`fatalError`/`DispatchQueue`, direct `Process(` in Swift UI, or unredacted secrets in logs/audit.
- Self-heal up to 3 attempts on check failures; never weaken tests or bypass linters.
