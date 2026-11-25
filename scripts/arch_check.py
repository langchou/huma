#!/usr/bin/env python3
"""Huma mechanical architectural & pure-VibeCoding guardrail checker.

Any rule that can be checked deterministically is enforced here rather than
left as advisory prose in AGENTS.md. Failures print exact file/line locations
and remediation guidance so coding agents can self-heal autonomously.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAX_PROD_LINES = 500
MAX_AGENTS_MD_LINES = 100
MAX_DOC_LINES = 100

RUST_TEST_MOD_RE = re.compile(r"(?m)^#\[cfg\(test\)\]\s*\n\s*(?:pub\s+)?mod\s+\w+\s*\{")
RUST_FORBIDDEN_PROD = (
    (re.compile(r"\.unwrap\s*\("), "Do not use `.unwrap()` in production Rust; propagate errors with `?` or handle `None`."),
    (re.compile(r"\.expect\s*\("), "Do not use `.expect()` in production Rust; return a typed error instead."),
    (re.compile(r"\btodo!\s*\("), "Do not leave `todo!()` placeholders in production code."),
    (re.compile(r"\bunimplemented!\s*\("), "Do not leave `unimplemented!()` in production code."),
    (re.compile(r"\bdbg!\s*\("), "Remove `dbg!()` debug macros; use `tracing` instead."),
)
RUST_LIB_FORBIDDEN_STDIO = (
    (re.compile(r"\bprintln!\s*\("), "Do not use `println!()` in library crates; use `tracing`."),
    (re.compile(r"\beprintln!\s*\("), "Do not use `eprintln!()` in library crates; use `tracing`."),
)
RUST_LIB_CRATES = {"huma-protocol", "huma-acp", "huma-store"}
RUST_ALLOW_RE = re.compile(r"(?m)^[ \t]*#\[allow\([^\]]+\)\]")

PROTOCOL_FORBIDDEN_UI_WORDS = (
    "sidebar",
    "capsule",
    "window",
    "theme",
    "color",
    "animation",
    "popover",
    "tooltip",
)

SWIFT_FORBIDDEN_PROD = (
    (re.compile(r"\btry!\b"), "Do not use `try!` in production Swift; propagate or handle errors explicitly."),
    (re.compile(r"\bfatalError\s*\("), "Do not use `fatalError()` in production Swift without an explicit `// huma-allow:` justification."),
    (re.compile(r"\bDispatchQueue\b"), "Prefer Swift Concurrency (`async`/`await`, `actor`, `@MainActor`) over `DispatchQueue` (or justify with `// huma-allow:`)."),
    (re.compile(r"\bProcess\s*\("), "UI layer must never spawn subprocesses directly; route agent execution through `humad`."),
    (re.compile(r"(?<![A-Za-z0-9_.])print\s*\("), "Do not use `print()` in production Swift; use `os.Logger`."),
    (re.compile(r"\bNSLog\s*\("), "Do not use `NSLog()` in production Swift; use `os.Logger`."),
)

MD_LINK_RE = re.compile(r"\[[^\]]+\]\((docs/[^)#]+)\)")


def mask_c_style_comments_and_strings(source: str) -> str:
    """Replace comments and string literals with spaces while preserving line numbers."""
    chars = list(source)
    n = len(source)
    i = 0
    while i < n:
        if source.startswith("//", i):
            end = source.find("\n", i + 2)
            end = n if end == -1 else end
            for k in range(i, end):
                chars[k] = " "
            i = end
            continue
        if source.startswith("/*", i):
            depth = 1
            end = i + 2
            while end < n and depth > 0:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            for k in range(i, end):
                if chars[k] != "\n":
                    chars[k] = " "
            i = end
            continue
        if source[i] == '"':
            end = i + 1
            while end < n:
                if source[end] == "\\":
                    end += 2
                    continue
                if source[end] == '"':
                    end += 1
                    break
                end += 1
            for k in range(i, min(end, n)):
                if chars[k] != "\n":
                    chars[k] = " "
            i = end
            continue
        i += 1
    return "".join(chars)


def strip_rust_test_modules(source: str) -> str:
    """Blank out `#[cfg(test)] mod ... { ... }` blocks while keeping line numbers intact."""
    masked = mask_c_style_comments_and_strings(source)
    chars = list(masked)
    pos = 0
    while match := RUST_TEST_MOD_RE.search(masked, pos):
        brace_start = match.end() - 1
        depth = 1
        idx = brace_start + 1
        while idx < len(masked) and depth > 0:
            if masked[idx] == "{":
                depth += 1
            elif masked[idx] == "}":
                depth -= 1
            idx += 1
        for k in range(match.start(), idx):
            if chars[k] != "\n":
                chars[k] = " "
        pos = idx
    return "".join(chars)


def line_number(source: str, offset: int) -> int:
    return source.count("\n", 0, offset) + 1


def has_huma_allow(raw_lines: list[str], line_no: int) -> bool:
    idx = line_no - 1
    if 0 <= idx < len(raw_lines) and "huma-allow:" in raw_lines[idx]:
        return True
    if idx - 1 >= 0 and "huma-allow:" in raw_lines[idx - 1]:
        return True
    return False


def has_comment_justification(raw_lines: list[str], line_no: int) -> bool:
    idx = line_no - 1
    if 0 <= idx < len(raw_lines) and "//" in raw_lines[idx]:
        return True
    if idx - 1 >= 0 and raw_lines[idx - 1].lstrip().startswith("//"):
        return True
    return False


def check_docs_map(errors: list[str]) -> None:
    agents_md = ROOT / "AGENTS.md"
    if not agents_md.exists():
        errors.append("Missing root AGENTS.md.")
        return
    content = agents_md.read_text(encoding="utf-8")
    agents_lines = len(content.splitlines())
    if agents_lines > MAX_AGENTS_MD_LINES:
        errors.append(
            f"AGENTS.md has {agents_lines} lines (limit {MAX_AGENTS_MD_LINES}). Keep AGENTS.md a lean routing map."
        )
    for rel_path in MD_LINK_RE.findall(content):
        if "<" in rel_path or "*" in rel_path:
            continue
        target = ROOT / rel_path
        if not target.exists():
            errors.append(
                f"AGENTS.md references missing doc `{rel_path}`. Create it or update the map."
            )

    docs_dir = ROOT / "docs"
    if docs_dir.exists():
        for doc in sorted(docs_dir.rglob("*.md")):
            lines = len(doc.read_text(encoding="utf-8").splitlines())
            if lines > MAX_DOC_LINES:
                errors.append(
                    f"{doc.relative_to(ROOT)} has {lines} lines (limit {MAX_DOC_LINES}). Keep agent docs telegraphic and concise."
                )


def check_cargo_layering(errors: list[str]) -> None:
    proto_toml = ROOT / "core/crates/huma-protocol/Cargo.toml"
    if proto_toml.exists():
        text = proto_toml.read_text(encoding="utf-8")
        for forbidden in ("tokio", "huma-acp", "huma-store", "humad", "rusqlite"):
            if re.search(rf"(?m)^\s*{re.escape(forbidden)}\b", text):
                errors.append(
                    f"{proto_toml.relative_to(ROOT)}: `huma-protocol` must stay pure data/wire types; forbidden dependency `{forbidden}`."
                )

    acp_toml = ROOT / "core/crates/huma-acp/Cargo.toml"
    if acp_toml.exists():
        text = acp_toml.read_text(encoding="utf-8")
        for forbidden in ("huma-store", "humad"):
            if re.search(rf"(?m)^\s*{re.escape(forbidden)}\b", text):
                errors.append(
                    f"{acp_toml.relative_to(ROOT)}: `huma-acp` must not depend upward on `{forbidden}`."
                )

    store_toml = ROOT / "core/crates/huma-store/Cargo.toml"
    if store_toml.exists():
        text = store_toml.read_text(encoding="utf-8")
        for forbidden in ("huma-acp", "humad"):
            if re.search(rf"(?m)^\s*{re.escape(forbidden)}\b", text):
                errors.append(
                    f"{store_toml.relative_to(ROOT)}: `huma-store` must not depend on `{forbidden}`."
                )


def check_rust_sources(errors: list[str]) -> None:
    crates_dir = ROOT / "core/crates"
    if not crates_dir.exists():
        return

    for path in sorted(crates_dir.glob("*/src/**/*.rs")):
        rel = path.relative_to(ROOT)
        raw = path.read_text(encoding="utf-8")
        raw_lines = raw.splitlines()
        prod = strip_rust_test_modules(raw)

        # Count non-blanked lines up to the last non-whitespace production line.
        prod_lines = prod.splitlines()
        prod_line_count = sum(1 for ln in prod_lines if ln.strip())
        # Enforce ceiling on total production span (excluding trailing test module).
        last_prod_idx = 0
        for idx, ln in enumerate(prod_lines, start=1):
            if ln.strip():
                last_prod_idx = idx
        if last_prod_idx > MAX_PROD_LINES:
            errors.append(
                f"{rel}: production code reaches line {last_prod_idx} (active lines={prod_line_count}), exceeding the {MAX_PROD_LINES}-line ceiling. Split into smaller cohesive modules."
            )

        for pattern, advice in RUST_FORBIDDEN_PROD:
            for m in pattern.finditer(prod):
                ln = line_number(prod, m.start())
                if not has_huma_allow(raw_lines, ln):
                    errors.append(f"{rel}:{ln}: {advice}")

        crate_name = rel.parts[2] if len(rel.parts) > 2 else ""
        if crate_name in RUST_LIB_CRATES:
            for pattern, advice in RUST_LIB_FORBIDDEN_STDIO:
                for m in pattern.finditer(prod):
                    ln = line_number(prod, m.start())
                    if not has_huma_allow(raw_lines, ln):
                        errors.append(f"{rel}:{ln}: {advice}")

        for m in RUST_ALLOW_RE.finditer(prod):
            ln = line_number(prod, m.start())
            if not has_comment_justification(raw_lines, ln):
                errors.append(
                    f"{rel}:{ln}: `#[allow(...)]` requires a comment directly above or inline explaining why."
                )

        # Keep huma-protocol free of UI-surface naming.
        if "huma-protocol" in rel.parts:
            lower_prod = prod.lower()
            for word in PROTOCOL_FORBIDDEN_UI_WORDS:
                for m in re.finditer(rf"\b{word}\b", lower_prod):
                    ln = line_number(lower_prod, m.start())
                    if not has_huma_allow(raw_lines, ln):
                        errors.append(
                            f"{rel}:{ln}: UI-surface term `{word}` found in `huma-protocol`. Use runtime-neutral domain names instead."
                        )


def check_swift_sources(errors: list[str]) -> None:
    swift_root = ROOT / "apps/macos/Sources"
    if not swift_root.exists():
        return

    for path in sorted(swift_root.rglob("*.swift")):
        rel = path.relative_to(ROOT)
        raw = path.read_text(encoding="utf-8")
        raw_lines = raw.splitlines()
        if len(raw_lines) > MAX_PROD_LINES:
            errors.append(
                f"{rel}: {len(raw_lines)} lines exceeds the {MAX_PROD_LINES}-line ceiling. Split into smaller focused views/types."
            )

        masked = mask_c_style_comments_and_strings(raw)
        for pattern, advice in SWIFT_FORBIDDEN_PROD:
            for m in pattern.finditer(masked):
                ln = line_number(masked, m.start())
                if not has_huma_allow(raw_lines, ln):
                    errors.append(f"{rel}:{ln}: {advice}")


def main() -> int:
    errors: list[str] = []
    check_docs_map(errors)
    check_cargo_layering(errors)
    check_rust_sources(errors)
    check_swift_sources(errors)

    if errors:
        print("Huma architecture guardrail violations found:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("✓ arch_check passed (layering, file-size ceiling, anti-cheat, doc links)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
