# AGENTS.md

## Project Overview

`rudesktop` is a Rust monorepo for desktop software. The repository currently contains `cpst`, clipboard command-line tools; `rubar`, a configurable status bar for Wayland and X11; and `shell-surface`, a reusable library for native desktop surfaces. More applications and shared crates may be added to the workspace. Do not assume that repository-wide code or instructions belong only to `rubar`.

## Repository Layout

- `Cargo.toml`: Cargo workspace members, shared package metadata, and shared dependencies.
- `Cargo.lock`: Lockfile for the workspace.
- `crates/cpst/`: The `cpst` package. Its `src/lib.rs` contains shared clipboard operations; `src/bin/` contains the `cpy` and `pst` commands. See `crates/cpst/README.md` for usage.
- `crates/rubar/`: The `rubar` application. Its `src/main.rs` is the entry point; `src/app.rs` manages the lifecycle; `src/config.rs` loads configuration; `src/render.rs` renders the bar; `src/widget/` contains widgets; and `src/provider/` contains platform and service integrations. See `crates/rubar/README.md` for usage and configuration.
- `crates/shell-surface/`: Shared surface and input abstraction. `src/lib.rs` defines the API, and `src/backend/` contains Wayland and X11 backends.
- `flake.nix`: Nix development shell and package definitions for `cpst` and `rubar`.
- `.github/workflows/ci.yml`: Workspace checks and release workflows for `cpst` and `rubar`.

When adding software, give it its own crate under `crates/` and register it in the root workspace manifest. Keep application-specific behavior in its application crate; move code into a shared crate when it has a clear reusable purpose. Update affected documentation, Nix definitions, and CI or release workflows as needed.

## Development Workflow

Run commands from the repository root. Use the Nix development shell when available:

```sh
nix develop
```

Format and validate the workspace before submitting changes:

```sh
cargo fmt --all --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

For a focused change, use `cargo check -p <package>` or `cargo test -p <package>` while developing, then run the relevant workspace checks. Run `rubar` with:

```sh
cargo run -p rubar
```

## Development Principles

- Do not make temporary workarounds that prioritize immediate behavior over a fundamental solution, or introduce fixes that are not root-cause fixes.
- When improving compatibility would reduce readability, prioritize readability over compatibility.
- Keep changes as small as necessary and do not combine them with unrelated refactoring. However, when the task itself is refactoring, changes do not need to be minimal; prefer broader refactoring where appropriate, as long as existing behavior is preserved.
- Do not silently ignore errors; handle them in a way that makes their causes identifiable whenever possible.
- Prioritize maintainability and runtime performance over ease of implementation.
- Avoid relying on external command execution whenever possible; when it is necessary, prefer using an external library or IPC instead.
- If the prompt is ambiguous or lacks necessary information, ask clarifying questions before proceeding.

## Git

After completing a task:

1. Run the relevant tests and linters.
2. Review the diff before committing.
3. Commit only changes made for the current task.
4. Use Conventional Commits for commit messages.
5. Write commit messages in English.
6. Do not push commits.
7. Do not amend, rebase, reset, or otherwise rewrite existing commits unless explicitly requested.
8. Do not include unrelated pre-existing working tree changes in the commit.
