#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

mode="${1:-all}"

run_arch() {
  echo "==> arch_check"
  python3 scripts/arch_check.py
}

run_rust() {
  if [ -f core/Cargo.toml ]; then
    echo "==> rust fmt"
    cargo fmt --manifest-path core/Cargo.toml --all --check
    echo "==> rust clippy"
    cargo clippy --manifest-path core/Cargo.toml --workspace --all-targets -- -D warnings
    echo "==> rust test"
    cargo test --manifest-path core/Cargo.toml --workspace
  else
    echo "==> skip rust (core/Cargo.toml not present)"
  fi
}

run_swift() {
  if [ -f apps/macos/Package.swift ]; then
    echo "==> swift test"
    swift test --package-path apps/macos
  else
    echo "==> skip swift (apps/macos/Package.swift not present)"
  fi
}

case "$mode" in
  all)
    run_arch
    run_rust
    run_swift
    echo "✓ all huma harness checks passed"
    ;;
  arch)
    run_arch
    ;;
  rust)
    run_rust
    ;;
  swift)
    run_swift
    ;;
  *)
    echo "usage: ./scripts/check.sh [all|arch|rust|swift]" >&2
    exit 2
    ;;
esac
