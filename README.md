# huma

A calm, native desktop workspace for parallel coding agents.

## architecture

- `core/` — cross-platform Rust daemon (`humad`) driving local coding agents via ACP and structured streams over Unix Domain Socket.
- `apps/macos/` — native macOS client (`SwiftUI` + `AppKit`), built for instant launch, 120Hz fluid transitions, and smooth streaming.

## development

This repository is agent-authored. Read [`AGENTS.md`](AGENTS.md) before making changes.

```bash
./scripts/check.sh
```

## license

[Apache-2.0](LICENSE)
