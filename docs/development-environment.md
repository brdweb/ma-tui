# Isolated development environment

The initial development host had no Rust toolchain, pkg-config or ALSA development
headers on PATH. A repository-local toolchain/sysroot was used rather than changing
system packages. `.tools/` and `target/` are ignored; they are not distributable
application files. Normal developer machines should use the setup in README.md.

## Reuse the existing local setup

From the repository root, in Bash:

```sh
export RUSTUP_HOME="$PWD/.tools/rustup"
export CARGO_HOME="$PWD/.tools/cargo"
export PATH="$CARGO_HOME/bin:$PWD/.tools/sysroot/usr/bin:$PATH"
export LD_LIBRARY_PATH="$PWD/.tools/sysroot/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PKG_CONFIG="$PWD/.tools/sysroot/usr/bin/pkgconf"
export PKG_CONFIG_PATH="$PWD/.tools/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$PWD/.tools/sysroot"
cargo test --all-targets --locked
```

These overrides are only for the isolated build shell; do not apply them globally
or use them when packaging for another Linux distribution.

## Setup actually used

- Downloaded the rustup shell installer from https://sh.rustup.rs to
  `.tools/rustup-init.sh`, then ran it with the local RUSTUP_HOME/CARGO_HOME above:
  `sh .tools/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain stable`.
- Installed `rustfmt` and `clippy` using `rustup component add rustfmt clippy`.
- Resulting compiler: `rustc 1.98.1 (48a229cea 2026-09-01)`.
- Used `apt-get download libasound2-dev libpkgconf3 pkgconf-bin`; moved the packages
  to `.tools/debs/` and extracted each using `dpkg-deb -x` into `.tools/sysroot/`.
- Supplied the missing runtime target for the extracted ALSA development symlink:
  `.tools/sysroot/usr/lib/x86_64-linux-gnu/libasound.so.2.0.0` points to the installed
  system ALSA runtime. This is a local sysroot repair, not a shipping requirement.
- `pkgconf --modversion alsa` returned `1.2.11`.
- `ma-tui --list-devices` found `alsa:null` only. Null output discards samples; it
  cannot establish physical-device compatibility or audible playback quality.

## Test discipline

The initial repository had no application tests/build to use as a pre-change
baseline. Tests were introduced incrementally and run RED then GREEN. Network
tests use local fixtures with synthetic tokens. The opt-in `audio_null` test uses
the real CPAL/SyncedPlayer output constructor on ALSA null, not an injected output.

PTY smoke tests run the actual executable and verify terminal restoration.
Ratatui redraws changed cells, so raw output does not necessarily contain a
contiguous search query/status string: decode the VT stream with pyte and assert
on its screen, rather than searching the raw byte history. Tests use `uv run
--with pyte python ...`; Python is not a runtime dependency of MA-TUI.

A successful localhost fixture exchange is not proof of live MA compatibility.
For stable qualification, validate actual MA 2.10.5 authentication, player registration,
physical-device sound, codec/format changes, server restart and multi-room sync
with explicit authorization. Retain the exact sendspin 0.3.7 pin during that work.

## Omarchy laptop (2026-09-08)

This checkout now builds natively on Omarchy 4.0.2 / Arch x86-64 with Rust/Cargo
1.98.0. No repository-local sysroot overrides are needed on this machine.
`cargo build --release --locked` uses the installed ALSA headers and libraries.
The available devices include `alsa:default` (PipeWire), `alsa:pipewire`,
`alsa:pulse`, null output and hardware outputs. Device enumeration alone does
not prove audible playback.

The required Rust format/lint/test checks, PTY demo/connection/settings tests,
CPAL null-output checks and a disposable real desktop keyring round-trip are the
laptop validation commands in README.md. The settings fixture uses a temporary
secret-tool double to prove saved-login reuse on a second application launch;
the separate opt-in keyring test saves/reads/removes a synthetic credential in
the actual desktop Secret Service. `NO_COLOR` is removed only from the theme
smoke-test environment so terminal color assertions can inspect RGB output.
The user's desktop theme is not changed by these tests.

Local installation uses the two `install -Dm...` commands in README.md. It adds
a user executable and desktop launcher, with no system service or public release.
Subsequent authorized tests confirmed live Music Assistant 2.10.2 connection,
endpoint registration, remote playback and sustained local output; the user also
confirmed local playback works. Broader hardware/codec coverage, live server
restart recovery and multi-room sync remain beta limitations in CHANGELOG.md.
