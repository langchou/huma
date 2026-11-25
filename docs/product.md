# Huma Product Spec (Core Invariants)

## Identity

Native desktop workspace for running 2–5 parallel repository coding agents via structured protocols (ACP + native streams) without terminal clutter or attention overload.

- Core runtime: cross-platform Rust daemon (`huma-core` / `humad`).
- Primary client: macOS native SwiftUI + AppKit (`apps/macos`), future native UIs attach to same daemon protocol.
- Agent-agnostic (BYOS): drives locally installed CLIs (`agy`, `opencode`, `claude`, `codex`, custom ACP). Preserves all native slash commands and interactive prompts.

## Core User Loops

1. **Zero-setup multi-repo**: open repo dir -> auto-register project -> auto-derive short semantic task title from session intent/updates (no manual naming required).
2. **Low-noise progress**: collapse tool calls and sub-agent noise into one live status line; keep main canvas calm and static until summary/decision content arrives.
3. **Zero-cold-start re-entry**: switching to any repo immediately shows clean, well-typeset latest assistant summary + objective status; user verifies directly in real runtime.
4. **Native multimodal input**: clipboard screenshot paste (`Cmd+V`) and file/image drag-and-drop passed natively as protocol attachments.
5. **Session persistence**: closing GUI window keeps daemon and all agent sessions running; menu bar + hotkey restores window instantly.

## Spatial Layout & Craft Gates

- **No permanent sidebar list**: main stage occupies >90% window width with generous whitespace.
- **Navigation**: minimal top project capsule rail + horizontal trackpad swipe + pinch/hotkey spatial zoom-out overview showing project cards (auto title + 1-line latest status).
- **Performance**: cold launch <150ms; 120Hz ProMotion transitions (180–240ms interruptible springs); zero abrupt layout jumps during streaming.
- **Smooth streaming**: adaptive token pacing + 60–80ms soft text fade-in + interpolated container height.

## Hard Non-Goals (v0.1)

- No full code editor, LSP, debugger, or heavy line-by-line diff IDE.
- No custom PTY terminal emulator.
- No Windows/Linux/mobile UI before macOS reaches v0.1 bar.
- No remote SSH cluster sync.
- No multi-model side-by-side racing workflows.
