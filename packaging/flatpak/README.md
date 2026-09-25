# Flatpak bundle

Download `ma-tui-v1.1.0-linux-x86_64.flatpak` and `SHA256SUMS` from
the GitHub release, then run:

```sh
sha256sum --ignore-missing -c SHA256SUMS
flatpak install --user ./ma-tui-v1.1.0-linux-x86_64.flatpak
flatpak run io.github.brdweb.MaTui
```

The installer obtains Freedesktop Platform 26.08 from Flathub if needed. This is
an x86-64 single-file bundle, not a Flathub listing or an update repository.
Replace an installed bundle with:
`flatpak install --user --reinstall ./ma-tui-*.flatpak`. To remove:
`flatpak uninstall --user io.github.brdweb.MaTui` (keeps settings by default).
The desktop entry uses your terminal emulator; the command above also works
inside an already-open terminal.

Set up the connection on first launch. Flatpak settings are separate from native
MA-TUI at `~/.var/app/io.github.brdweb.MaTui/config/ma-tui/config.toml`. Saved login
uses the desktop Secret Service keyring. The bundled `secret-tool` helper is
built from libsecret 0.21.7; its complete upstream source and LGPL notice are
included under `/app/share/licenses/ma-tui`. Other libraries come from the runtime.
The new profile gets its own speaker identity; close native MA-TUI before using
the Flatpak as your laptop speaker to avoid two endpoints.

Permissions enable the network for Music Assistant, PulseAudio for desktop audio
(including PipeWire's PulseAudio compatibility service), and the Secret Service
D-Bus name for login storage. The Flatpak also owns
`org.mpris.MediaPlayer2.ma_tui` so desktop media controls can discover MA-TUI,
and talks to `org.freedesktop.Notifications` for optional track notifications.
Flatpak's PulseAudio permission also permits audio input, although MA-TUI only
opens output streams. Read-only access to `~/.local/state/omarchy/current`
follows modern Omarchy theme updates without access to the rest of your home or
native MA-TUI configuration. Legacy/custom Omarchy theme locations are not
exposed automatically. These specific D-Bus permissions do not grant full
session-bus access. The runtime's default ALSA output routes through PulseAudio;
use the desktop mixer to select the physical output.

Realtime scheduling is provided by the host audio stack. If it needs RTKit,
install your distribution's `rtkit` package on the host; the Flatpak does not
bundle or start the system daemon. PipeWire also supports configured realtime
limits, and sandbox clients can use the Realtime portal where their audio
backend supports it. MA-TUI does not grant full system-bus access for RTKit.
See [local audio troubleshooting](../../docs/audio-troubleshooting.md).
The bundle includes it at `/app/share/doc/ma-tui/audio-troubleshooting.md`.

## Build and verify

Prerequisites: a native release executable (local or GitHub-built), `flatpak`, installed
`org.freedesktop.Platform//26.08`, C compiler, `pkg-config`, libsecret development
headers and `desktop-file-validate`. No flatpak-builder or compiler SDK is needed.

```sh
cargo build --release --locked
python3 packaging/arch/stage.py
python3 packaging/flatpak/build.py
flatpak install --user --noninteractive .tools/flatpak-package/ma-tui-v1.1.0-linux-x86_64.flatpak
flatpak run io.github.brdweb.MaTui --version
flatpak run io.github.brdweb.MaTui --demo --snapshot
flatpak run io.github.brdweb.MaTui --list-devices
python3 packaging/flatpak/verify.py
```

The verifier needs Cargo, `uv`, an unlocked desktop keyring and a working
PulseAudio output. It runs local protocol/terminal fixtures and a silent real
audio stream; it never connects to your Music Assistant server. Do not run
another instance of this Flatpak during the SIGTERM fixture.

For a verified GitHub Actions build, set `MA_TUI_CI_BUILD` to the extracted
`ma-tui-release-build` artifact directory instead of running the local Cargo
build. Keep that setting for Arch staging, Flatpak building and final bundling;
the scripts require its source commit, lockfile and executable hashes to match.

The builder stages only allowlisted binary, helper, docs and notices, verifies
its pinned source download, and records binary/helper/runtime identities. It
wraps the native binary and compiles the helper locally; it does not claim a
reproducible or signed build. The application branch is `stable`.
See [release procedure](../../docs/releasing.md) for publication checks.

Official references: [single-file bundles](https://docs.flatpak.org/en/latest/single-file-bundles.html),
[sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html),
and [libsecret source](https://download.gnome.org/sources/libsecret/0.21/).
