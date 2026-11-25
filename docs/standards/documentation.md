# Documentation Standard

Audience: AI coding agents. Optimize for token density and precision, not human prose.

## Scope & Layout

- `AGENTS.md`: root routing map + non-negotiable mandates (<= 100 lines).
- `docs/standards/*.md`: repository rules (`documentation.md`, `development.md`, `logging-audit.md`).
- `docs/product.md`, `docs/architecture.md`, `docs/roadmap.md`: Huma system truth.
- `docs/specs/<feature>.md`: pre-implementation feature contracts.

## Writing Rules

- MUST write in concise, telegraphic English bullets using `MUST` / `MUST NOT` / exact paths / exact commands / exact schemas.
- MUST focus exclusively on Huma. MUST NOT mention competitors, market comparisons, backstory, or motivational prose.
- MUST keep one fact in one canonical doc (Single Source of Truth). Cross-reference by relative path; never duplicate rule lists.
- MUST stay within line budgets: `AGENTS.md` <= 100 lines; every `docs/**/*.md` <= 100 lines.
- MUST NOT leave stale, contradictory, or orphan docs. Delete superseded docs immediately.

## Required Templates

- **Standard (`docs/standards/*.md`)**: `Scope`, `Rules`, `Mechanical Gates`.
- **Spec (`docs/specs/<name>.md`)**: `Goal`, `Non-Goals`, `Contract`, `Verification`.
- **System Doc (`docs/*.md`)**: `Invariants`, `Interfaces`, `Boundaries`.

## Sync Mandate

- Any diff changing wire schemas, CLI commands, module layout, or invariants MUST update the canonical doc in the same task.

## Mechanical Gates

- `scripts/arch_check.py` enforces:
  - `AGENTS.md` <= 100 lines.
  - Every `docs/**/*.md` <= 100 lines.
  - All `docs/...` links in `AGENTS.md` resolve to existing files.
