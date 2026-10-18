# AGENTS.md

## Project Overview

`rubar` is a Rust-based status bar application. It renders widgets such as a clock, battery status, workspace information, and audio volume, and loads its configuration through the project's configuration module.

## Repository Layout

- `src/main.rs`: Application entry point.
- `src/app.rs`: Application lifecycle and shell integration.
- `src/config.rs`: Configuration loading and parsing.
- `src/render.rs`: Rendering support.
- `src/widget/`: Individual status bar widgets and their providers.
- `Cargo.toml`: Rust package metadata and dependencies.
- `flake.nix`: Reproducible development shell definition.

## Development Workflow

Use the Nix development shell when available:

```sh
nix develop
```

Format and validate the project with Cargo before submitting changes:

```sh
cargo fmt --check
cargo check
cargo clippy
cargo test
```

Run the application locally with:

```sh
cargo run
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
