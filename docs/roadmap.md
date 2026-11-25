# Huma MVP Roadmap (v0.1)

## Phase 0 — Core Harness & Protocol Verification

- Scope: `core/` workspace (`huma-protocol`, `huma-acp`, `humad`) + smoke CLI.
- Deliverables:
  - `agy acp --stdio` full lifecycle: `initialize` -> `session/new` -> prompt (text + image) -> streaming updates -> slash commands -> cancel -> permission reply.
  - Second driver verified (`opencode acp` or `claude` stream-json).
  - `humad` Unix Socket server managing >= 3 concurrent repo sessions with multi-client event fanout.
- Exit gate: `./scripts/check.sh` green + live 3-project smoke test passes with client detach/reattach keeping sessions alive.

## Phase 1 — Single-Project Native macOS Craft

- Scope: `apps/macos` (`HumaClient`, `HumaRenderer`, `HumaApp`) attached to `humad`.
- Deliverables:
  - Auto-connect / auto-launch `humad` on app open (<150ms UI paint).
  - Smooth Ink 120Hz streaming Markdown renderer (zero chunk jolt, zero scroll fight).
  - Single-line collapsible activity capsule above response canvas.
  - Floating command bar: `Cmd+V` screenshot paste, drag-drop image/file attachments, `/` slash command completion, inline permission prompt UI.
  - Calm light + dark themes.
- Exit gate: `./scripts/check.sh` green + half-day real single-repo session passes craft gates.

## Phase 2 — Multi-Project Spatial Workspace

- Scope: `huma-store` (SQLite WAL) + multi-project UI in `apps/macos`.
- Deliverables:
  - Persistent repo registry, auto semantic titles, latest turn summaries, session restore.
  - Minimal top project capsule rail + horizontal swipe project switching.
  - Pinch / hotkey spatial zoom-out overview (project cards with auto title + 1-line status) with spring zoom-in.
  - Zero-cold-start landing showing latest summary immediately on switch.
- Exit gate: `./scripts/check.sh` green + 3–5 concurrent repos used smoothly end-to-end.

## Phase 3 — Daily Driver Polish

- Menu bar resident mode (close window keeps daemon running) + global summon hotkey.
- Auto-discovery of installed local ACP agents.
- Quick-open repo in external editor / terminal.
- Signed macOS `.dmg` packaging.
