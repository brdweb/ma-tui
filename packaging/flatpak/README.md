# Flatpak bundle

The current candidate is MA-TUI 1.1.1, targeting Music Assistant stable 2.10.5.
Candidate qualification evidence and remaining desktop/publication gates are
documented in `docs/releasing.md`.

After an authorized release is published, download
`ma-tui-v1.1.1-linux-x86_64.flatpak` and `SHA256SUMS` from
the GitHub release, then run:

```sh
sha256sum --ignore-missing -c SHA256SUMS
flatpak install --user ./ma-tui-v1.1.1-linux-x86_64.flatpak
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
flatpak install --user --noninteractive .tools/flatpak-package/ma-tui-v1.1.1-linux-x86_64.flatpak
flatpak run io.github.brdweb.MaTui --version
flatpak run io.github.brdweb.MaTui --demo --snapshot
flatpak run io.github.brdweb.MaTui --list-devices
python3 packaging/flatpak/verify.py
```

The verifier needs Cargo, `uv`, an unlocked desktop keyring and a working
PulseAudio output. It runs local protocol/terminal fixtures and a silent real
audio stream; it never connects to your Music Assistant server. Do not run
another instance of this Flatpak during the SIGTERM fixture.

### Disposable container build and verification

Build inside a conventional Linux container, **not a host Nix shell**. A
Nix-host-built `ma-tui` or `secret-tool` can require a `/nix/store` ELF loader
that does not exist in the Flatpak runtime. Reuse a container-built executable
with glibc no newer than Platform 26.08, or the matching GitHub CI artifact.

The Ubuntu 24.04 workflow below reuses the staged native candidate and its
matching repository-local GNU Rust toolchain under `.tools/rustup`. If the
candidate is not staged, run `python3 packaging/arch/stage.py` in its native
build container first. Do not rebuild the qualified release executable here.

```sh
mkdir -p .tools/flatpak-package
docker --host unix:///run/user/1001/docker.sock run --rm \
  --name ma-tui-flatpak-verify \
  --security-opt seccomp=unconfined \
  --security-opt systempaths=unconfined \
  -e BUILD_UID="$(id -u)" \
  -v "$PWD:/source:ro" \
  -v "$PWD/.tools/flatpak-package:/output" \
  ubuntu:24.04 bash /source/packaging/flatpak/verify-container.sh
```

Both options are required on the `hermes` rootless Docker daemon: its default
seccomp profile blocks `unshare(CLONE_NEWUSER)`, while masked `/proc` prevents
Bubblewrap from mounting proc. Either exception alone is insufficient;
`--cap-add SYS_ADMIN` is not a replacement. Apply these exceptions **only to
this disposable Flatpak container**, never daemon-wide or to other containers.

The script installs Flatpak, compiler/libsecret headers, Python, Cargo and
`uv`; the matching Rust toolchain takes precedence over distribution Cargo.
The builder UID matches the invoking user's numeric UID. Rootless bind mounts
map their owner to container root, so the script copies allowlisted repository
and candidate inputs to a builder-owned workspace instead of changing mount
ownership. It builds and installs the bundle as that non-root user.

All services and state are private to the container: a system bus,
`dbus-run-session`, a GNOME Secret Service unlocked with a throwaway password,
and PulseAudio with the `ma_tui_fixture` null sink as default output. No real
home, keyring, configuration, audio socket or Music Assistant server is used.
It runs `--version`, `--demo --snapshot`, `--list-devices`, then `verify.py`
through `FLATPAK VERIFIED`. Only a successful run exports the bundle,
`BUILDINFO.json` and `VERIFIED.json` into `.tools/flatpak-package`.

The null sink qualifies sandbox audio access and the silent stream test, not
audible desktop output. Physical playback, desktop media keys, notifications
and desktop integration still require separate qualification. A dirty
candidate's verification record is not final committed-source provenance.


For a verified GitHub Actions build, set `MA_TUI_CI_BUILD` to the extracted
`ma-tui-release-build` artifact directory instead of running the local Cargo
build. Keep that setting for Arch staging, Flatpak building and final bundling;
the scripts require its source commit, lockfile and executable hashes to match.

The builder stages only allowlisted binary, helper, docs and notices, verifies
its pinned source download, and records binary/helper/runtime identities. It
wraps the native binary and compiles the helper locally; it does not claim a
reproducible or signed build. The application branch is `stable`.
See [release procedure](../../docs/releasing.md) for publication checks.

Bundle names and recorded versions are derived from `Cargo.toml`, not these
example filenames. Final release bundling requires a clean committed tree, a
matching restaged executable, verified Arch package and installed Flatpak bundle
verification record. Rebuild/reverify when the final build input changes; old
package artifacts or verification records do not qualify this candidate.

Official references: [single-file bundles](https://docs.flatpak.org/en/latest/single-file-bundles.html),
[sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html),
and [libsecret source](https://download.gnome.org/sources/libsecret/0.21/).
