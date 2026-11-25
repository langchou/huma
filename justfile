# Huma — Unified Agent Verification Harness

set shell := ["zsh", "-cu"]

check:
    ./scripts/check.sh all

arch-check:
    ./scripts/check.sh arch

rust-check:
    ./scripts/check.sh rust

swift-check:
    ./scripts/check.sh swift

rust-fmt:
    cargo fmt --manifest-path core/Cargo.toml --all
