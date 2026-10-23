# AGENTS.md

Instructions for coding agents in this repository (Cursor cloud agents, Codex, Claude Code). Codex (`chatgpt-codex-connector`) is the only automatic pull request reviewer.

Personal or machine-specific notes never go in tracked files. Claude Code auto-loads the gitignored `CLAUDE.local.md`; Codex users put personal guidance in `~/.codex/AGENTS.md`; `AGENTS.local.md` is a gitignored scratch file that tools don't load automatically.

## Project

MA-TUI (`ma-tui`, v1.1.1, MIT) is a Linux terminal controller and local player for Music Assistant 2.10.5. It is one Rust 2021 binary: Ratatui and crossterm draw the interface, Tokio runs async work, reqwest (rustls) and tokio-tungstenite call the Music Assistant HTTP API and `/ws` event stream, embedded `sendspin = "=0.3.7"` and CPAL play audio locally, and zbus 6 exposes MPRIS. There is no companion player process and nothing is deployed as a service.

- `src/` — library and binary (`api`, `audio`, `controller`, `events`, `mpris`, `terminal_ui`, `ui`, and the modules beside them).
- `tests/` — Rust tests, Python PTY smokes, and `tests/packaging_build_input.py`.
- `packaging/` — Arch staging, Flatpak build and verify, `ci_build.py`, and `release.py`. Shipping steps are in `docs/releasing.md`.
- `docs/` — `architecture.md`, `development-environment.md`, `releasing.md`, `audio-troubleshooting.md`.
- `.github/workflows/ci.yml` — the only GitHub Actions workflow. `Cargo.lock` is committed.

## Commands

`.github/workflows/ci.yml` has two jobs. Run the commands below as written.

**Rust quality gates** on every pull request. The job installs `libasound2-dev` first; a local build needs the same ALSA headers (see README.md).

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python3 tests/packaging_build_input.py
```

That job builds and collects the release artifact only when the ref is `main` and the event is `push` or `workflow_dispatch`. Pull-request CI does not run these:

```sh
cargo build --release --locked
python3 packaging/ci_build.py
```

`packaging/ci_build.py` requires a clean tree and the GitHub Actions environment of a main push or manual dispatch (ref, event, SHA, repository, and origin). It is not a pull-request check.

**RustSec advisory audit** on every pull request. The job fails on advisories and does not open issues:

```sh
cargo install cargo-audit --version 0.22.2 --locked
cargo audit --file Cargo.lock
```

`python3 tests/packaging_build_input.py` is the only Python test CI runs. It is a `unittest` module. Arch staging, Flatpak build/verify, and `python3 packaging/release.py` are release steps in `docs/releasing.md`, not CI jobs.

README.md documents three PTY smokes that CI does not run. After `cargo build --release --locked`, use them when terminal behavior matters:

```sh
uv run --with pyte python tests/terminal_smoke.py target/release/ma-tui
uv run --with pyte python tests/connected_smoke.py target/release/ma-tui
uv run --with pyte python tests/settings_smoke.py target/release/ma-tui
```

`docs/development-environment.md` describes an optional toolchain under `.tools/`. Do not export those variables when packaging for another distribution.

## Definition of done

- Start from a GitHub Issue that states the goal, non-goals, acceptance criteria, and how to prove them.
- The four pull-request Rust quality gates commands, plus `cargo audit --file Cargo.lock` from cargo-audit 0.22.2, pass locally. Paste that output in the PR body. If a command could not be run, name it and why.
- Every bug fix adds a regression test that fails before the fix.
- A change visible in the TUI includes a terminal screenshot or recording.
- The PR links the Issue and states what was not done.

## Boundaries

- Open draft pull requests only. Never push to `main`.
- Never force-push, delete branches or tags, or create tags or releases. Releases follow `docs/releasing.md` and need explicit authorization.
- No major dependency bumps. No edits to `.github/`, packaging or release scripts, or secrets, unless the Issue asks for that change.
- Never weaken or delete a test to make CI pass.
- Keep a pull request to about 400 changed lines. Split work that would exceed that.
- When a requirement is ambiguous, ask in the PR instead of guessing.

## Commits and PRs

Use Conventional Commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `build(deps):`, `chore:`. The PR title uses the same form, because squash merges take the title as the commit subject. Never add `Co-Authored-By` or any other AI co-author or attribution trailer to a commit message or a PR body.

## Review guidance

Codex (`chatgpt-codex-connector`) is the only automatic pull request reviewer. Claude Code is optional: someone calls it in with an `@claude` comment, where configured, for risky PRs only, and it never runs automatically. Reviews focus on async and MPRIS correctness, Music Assistant API handling (bare JSON from `/api`, `/ws` replies matched by `message_id`, queue identity kept apart from player identity, no credentials or response bodies in errors), terminal rendering and restoration, and packaging integrity (staged assets, `Cargo.lock`, third-party notices). The agent that opened the PR fixes review findings.

## Review and merge policy

- Bots never post GitHub approvals.
- A PR merges only after CI is green and every Codex review thread is fixed and resolved.
- The triage bot may auto-merge only patch and minor dependency bumps, security fixes, docs, tests, and small fixes under about 50 changed lines.
- Other bots and agents merge only work the maintainer has explicitly approved.
- Everything else waits for the maintainer: major version bumps, auth, payments, database migrations, CI or release configuration, secrets, and large or ambiguous PRs.
- Squash merge. Commits carry no Co-Authored-By or other AI co-author trailers.

## Learning

Read [`docs/agent-mistakes.md`](docs/agent-mistakes.md) before starting. When a reviewer catches a repeatable mistake, add an entry to that file.
