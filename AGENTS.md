# MA-TUI

Rust/Ratatui Music Assistant terminal controller with embedded Sendspin local audio. Current release: v1.0.0 (2026-09-23). Remote: https://github.com/brdweb/ma-tui.git

## Architecture

Async Music Assistant WebSocket client with optional CPAL local audio output. Config at `$XDG_CONFIG_HOME/ma-tui/config.toml`. Credentials saved to the desktop Secret Service via `secret-tool`. PTY smoke tests use Python/pyte against local HTTP/WebSocket fixture servers with synthetic tokens and null audio.

## Key directories

- `src/` — Rust source: TUI renderer, MA WebSocket client, audio pipeline, settings
- `tests/` — Python PTY smoke tests and fixture servers
- `packaging/arch/`, `packaging/flatpak/` — distribution packaging

## Build and verify

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
uv run --with pyte python tests/terminal_smoke.py target/release/ma-tui
uv run --with pyte python tests/connected_smoke.py target/release/ma-tui
uv run --with pyte python tests/settings_smoke.py target/release/ma-tui
```

Offline UI preview (no server, no audio): `./target/release/ma-tui --demo`

## Safety rules

- Do not connect to a live Music Assistant server without explicit user authorization.
- Do not access the desktop keyring (Secret Service) during automated testing. Tests use fixture servers with synthetic tokens.
- Do not attempt real audio output or speaker registration without explicit authorization. Tests use null audio.
- PTY smoke tests require an actual TTY; do not pipe input/output in a way that bypasses PTY allocation.
