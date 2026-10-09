# Releases

Releases require explicit user authorization. The latest stable release is
`v1.1.1`, targeting Music Assistant 2.10.5; the previous release is `v1.1.0`.
`v0.9.0` was the first as ma-tui. `v0.1.0-beta.2` was
published as local-matui before the rename, and `v0.1.0-beta.1` under that same
former name and withdrawn the same day; its tag and assets were deleted rather
than rewritten. MA-TUI is MIT licensed; include the root LICENSE in all new
packages alongside third-party notices. A prerelease tag must not be labeled as
a stable/latest release; a release that is one may be.

1. Update Cargo.toml/Cargo.lock and CHANGELOG.md on the feature branch. Run
   `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
   `cargo test --all-targets --locked`, and `cargo build --release --locked`.
2. Run the release binary through the demo, connected-controller and both settings
   PTY fixtures listed in README.md. Keep live-test authorization and known gaps
   explicit; fixture results do not demonstrate acoustic latency or multi-room sync.
3. Run `python3 packaging/arch/stage.py`, then the disposable Arch package
   verification command in packaging/arch/README.md. It must install, verify,
   exercise and uninstall the package successfully before publication.
   Build `python3 packaging/flatpak/build.py`, install the resulting bundle, and
   verify sandbox startup, PTY restoration, keyring round trip, theme access and
   the silent default-output fixture. See packaging/flatpak/README.md.
4. Commit and push the release preparation, review/merge the PR into main, and
   verify that the merged source tree equals the tested feature tree. Use a clean
   main checkout. For a GitHub-built release, wait for that exact main commit
   to pass CI and the advisory audit, then download its `ma-tui-release-build`
   artifact and extract its tarball into a private staging directory. Export
   `MA_TUI_CI_BUILD` to that directory, rerun package staging/build/verification
   with the downloaded executable, and run `python3 packaging/release.py`.
   The scripts validate CI commit/tree, version, lockfile, executable hash and
   compiler/runtime notice identity. BUILDINFO records the GitHub run and the
   actual compiler. Without that variable, a local `cargo build --release
   --locked` remains supported. The bundler checks binary/package identity.
5. Create and push an annotated `v<version>` tag at that main commit. Create a
   GitHub draft release with `--verify-tag`, adding `--prerelease --latest=false`
   for a beta and neither for a release that is not one. Upload only the
   versioned dist directory's six assets, and publish it once complete.
6. Download the hosted assets into a fresh directory, verify SHA256SUMS and the
   embedded executable version/hash, and confirm the tag, source commit, intended
   repository visibility and prerelease flag. Never claim signing or reproducible builds
   unless those checks were actually performed.

Artifacts include the native Linux archive, Arch package, Flatpak bundle, source archive,
BUILDINFO.json and SHA256SUMS. The binary archives include third-party notices
from the locked Cargo graph and Rust runtime. Personal configuration, tokens,
keyring data, media and `.tools/` must never enter release assets.

The Arch version removes Cargo's prerelease hyphen, which a release without one
does not have: `0.9.0` is used as is. The package is unsigned and no AUR or
distribution-repository publication is implied. No service is deployed. The
Flatpak application branch is `stable`; it was `beta` while the releases were,
and a ref is not upgraded across branches.

## 1.1.1 publication (2026-10-09)

- Published [v1.1.1](https://github.com/brdweb/ma-tui/releases/tag/v1.1.1)
  as stable/latest at 14:51:13 UTC, with six assets: native archive, source
  archive, Arch package, Flatpak bundle, `BUILDINFO.json` and `SHA256SUMS`.
- The annotated tag resolves to `ef4c9237fa056603c1f8e796e60ba36a07230f2f`,
  source tree `ecca5c35a574aa8cc1e080108c5589f62bf0a24b`.
  [Main CI run 37945356369](https://github.com/brdweb/ma-tui/actions/runs/37945356369)
  passed quality checks, packaging tests, optimized build and RustSec audit.
  Every final wrapper used that run's release-build artifact through
  `MA_TUI_CI_BUILD`, not the earlier dirty candidate.
- Final native executable SHA256:
  `e2aa6932fea5b4577e4446907a646920be699f5b7267df106b5833a20c38fb0c`.
  Native PTY/settings/controller fixtures and Arch install, integrity,
  startup, PTY/controller and uninstall checks passed.
- Final Flatpak bundle SHA256:
  `2895de42a66feb8feec42928a46bc2882910f8ffdc6c790a5e0d220305cf6fda`.
  The corrected disposable-container workflow again reached `FLATPAK VERIFIED`.
  Final `BUILDINFO.json` and `VERIFIED.json` are retained in
  `.tools/flatpak-package/`; the full output is
  `.tools/release-1.1.1/final-flatpak-verification.log`.
- Downloaded all hosted assets after publication. Every checksum passed;
  native/Arch executable, source lockfile and Flatpak identities matched the
  final CI provenance. The downloaded native executable passed version and
  demo-snapshot checks inside Ubuntu 24.04. Remote tag, stable/latest flags
  and the six hosted asset names were verified.
- Audio verification used a disposable null sink. Audible desktop playback,
  media keys, notifications and real desktop integration remain unqualified.
  The published assets are unsigned and are not reproducible-build attestations.

## 1.1.1 preparation (2026-10-09)

Music Assistant [2.10.5](https://github.com/music-assistant/server/releases/tag/2.10.5)
is already stable (published 2026-10-02); this release is not waiting for an
upstream stable promotion. The candidate retains Sendspin 0.3.7 and excludes
2.11-only features. Candidate qualification did not publish, tag or install
on the host. Final GitHub publication and Omapak submission were subsequently
authorized on 2026-10-09, with the desktop coverage limits below retained.

### Candidate evidence

- Built with the repository-local Rust 1.98.1 toolchain in a disposable Ubuntu
  24.04 container. Formatting and strict Clippy passed; ordinary Rust targets
  passed with 224 tests and 14 ignored. The locked optimized executable reports
  `ma-tui 1.1.1`; its native glibc requirement is 2.39.
- All five native PTY fixtures passed: demo quit, demo SIGTERM, connected
  controller, password settings and token settings. The settings fixture still
  emits its previously documented background assertions for unsupported `/ws`
  requests; these passes do not qualify its event path.
- The real CPAL null-output integration and three opt-in device-output tests
  passed: callback progress/buffer size, the full advertised PCM buffer, and
  delay resets before audio/after clock reset.
- All 13 packaging build-input tests passed. The staged candidate Arch package
  passed install, integrity, version, desktop/icon/license checks, demo,
  device enumeration, terminal/controller fixtures and removal in a disposable
  Arch container. This was a dirty precommit candidate, not final-source
  provenance or verification of a GitHub CI artifact.
- RustSec audit scanned 403 locked dependencies with the fetched advisory
  database and reported no vulnerabilities. Its crates.io index refresh emitted
  a registry-path warning; this is not evidence of registry freshness.
- The earlier disposable-container attempt installed Platform 26.08, but
  failed before package construction with Bubblewrap's
  `Creating new namespace failed: Operation not permitted`, even with private
  system and session buses. No artifact was qualified by that attempt.
  The cause was the rootless Docker container defaults, not the VM: seccomp
  blocked `unshare(CLONE_NEWUSER)` and masked `/proc` prevented mounting proc.
  Both per-container options, `--security-opt seccomp=unconfined` and
  `--security-opt systempaths=unconfined`, are required. Never apply them
  daemon-wide or to other containers; `--cap-add SYS_ADMIN` is not a substitute.
- The corrected Ubuntu 24.04 container run used both options only on that
  disposable container. A non-root builder (UID 1001) reused the qualified
  native executable; its SHA256 remained
  `42fc9d593de0638c58ef8ef09d9625b0d94fc65e06b85a853992ffd97fed3b72`.
  `python3 packaging/flatpak/build.py` built the helper with Ubuntu GCC 13.3,
  installed the bundle with `flatpak install --user --noninteractive`, and
  passed `flatpak run io.github.brdweb.MaTui` with `--version`,
  `--demo --snapshot` and `--list-devices`.
- `python3 packaging/flatpak/verify.py` reached `FLATPAK VERIFIED`: installed
  binary/helper and documentation identities, native-config isolation,
  synthetic GNOME keyring store/lookup/clear, PTY quit and SIGTERM, local
  controller fixtures and the silent default-output test all passed. The
  private session used `dbus-run-session` and PulseAudio's `ma_tui_fixture`
  null sink. No Omarchy theme was present, so its conditional identity check
  did not run. Audible playback, media keys, notifications and real desktop
  integration were not qualified.
- The initial candidate bundle was
  `.tools/flatpak-package/ma-tui-v1.1.1-linux-x86_64.flatpak`, SHA256
  `3b602b408d2017b0cf9c2f2a75f7598ef1821157448a3afe52794f634d35bde0`.
  Its local staging records have since been replaced by the final CI-derived
  bundle and matching records described above. The original successful
  candidate-container output remains at
  `.tools/release-1.1.1/flatpak-container-verification.log`. The disposable
  container was removed; its homes, buses, keyring and audio service were
  never mounted from the host. The upstream helper emitted an unused-result
  compiler warning, and ALSA device enumeration warned about absent physical
  cards/OSS; neither prevented the verified PulseAudio default-output test.
- A disposable official Music Assistant 2.10.5 container, API schema 65,
  passed real-TUI authentication, Sendspin registration, synthetic WAV playback
  accepted by ALSA null, advancing queue position, an external volume update
  reaching the TUI, queue clear via F4/c, and terminal restoration. The image
  digest was `sha256:28023f8c0d96ca2496391f3218a6d70d7ef3ba84db846b3a3f3cd9ce7ef8aedc`.
  Only a synthetic account/media and private temporary server data were used;
  no existing Music Assistant server or physical speaker was accessed.

Native, Arch and target-server logs are retained under
`.tools/release-1.1.1/`. The disposable server was stopped and removed. The
synthetic HTTP audio server logged a connection reset during stream shutdown;
all target-server assertions passed. Null output is not audible-playback proof.

### Publication gates

1. Desktop/provider qualification remains incomplete: audible local output,
   the intended remote speaker, media keys, notifications, codecs, restart
   and multi-room coverage must not be claimed from container/null output.
   Publication was authorized with these limitations explicitly documented.
2. Build and verify the Flatpak against Platform 26.08. This gate can be met
   in a disposable container using both sandbox options above, a private
   session bus, unlocked Secret Service and PulseAudio-compatible null output;
   see `packaging/flatpak/README.md`. Container null output does not qualify
   audible desktop playback, media keys, notifications or desktop integration.
   Native and Arch verification do not satisfy the Flatpak gate.
3. With authorization, commit/merge the preparation, obtain a passing CI/audit
   run for that exact main commit, and use its release-build artifact. Restage
   and reverify packages against that clean committed source, including the
   final documentation. Candidate packages do not establish final provenance.
4. Run the existing release bundler only after both package gates pass. It
   requires clean source and matching package identities/Flatpak verification.
   Tagging, upload, publication and hosted-asset checks follow the authorized
   procedure above. Omapak builds independently from the tag through its
   catalog recipe; submission does not establish publication until its human
   maintainer merges and the repository build/deployment succeeds.

## 1.0.0 validation (2026-09-23)

The release was prepared and tested locally, on a precommit candidate, before
the release commit. GitHub publication uses the optimized executable built by
the main-branch CI run, followed by package verification of that exact
artifact, as for 0.9.3.

- Formatting and strict Clippy passed; all ordinary Rust targets passed with
  187 tests and 14 ignored. The optimized binary reports `ma-tui 1.0.0` and shows
  `MA-TUI v1.0.0` in the header.
- All five native terminal fixtures passed: quit, SIGTERM, connected controller
  (now also favourite, sort, start radio, add to playlist and save queue), password
  settings and token settings.
- A throwaway session-bus fixture drove the native binary and the installed
  Flatpak over MPRIS: identity, status, title/artists/album metadata and volume
  read correctly; PlayPause, Next, Pause and Volume reached the fixture server;
  the bus name was released on quit. The Flatpak's session-bus policy is limited
  to owning `org.mpris.MediaPlayer2.ma_tui` and talking to
  `org.freedesktop.Notifications` and `org.freedesktop.secrets`.
- The staged build passed the disposable Arch container verification:
  install, integrity, startup, PTY, HTTP fixtures and removal.
- The Flatpak built by `packaging/flatpak/build.py` passed
  `packaging/flatpak/verify.py` in the user installation: binary/helper
  identity, version/demo/devices, native-config isolation, Omarchy theme
  identity, a disposable real keyring round trip, quit/SIGTERM, the connected
  controller fixture and the silent real default-output test.
- RustSec `cargo audit` of the locked graph (403 crates, including the new zbus
  dependencies) reported no advisories.
- The user ran the Flatpak candidate against their Music Assistant server and
  confirmed the new browsing, favourites, radio and playlist features. Media
  keys were not exercised by hand (no media keys on the test keyboard); MPRIS
  was verified over the session bus as above.

Omapak builds its own Flatpak from the tagged source through its catalog
recipe (`apps/io.github.brdweb.MaTui/` in outcrop-labs/omapak); the GitHub
release bundle is not that build.

## 0.9.3 validation (2026-09-17)

The release was prepared and tested locally before GitHub publication was
authorized. The audio changes keep isolated
ALSA underruns and the exact transient `snd_pcm_avail_delay` I/O error connected
when new callbacks prove recovery. Persistent errors, repeated recovery
episodes, and stalled callbacks still trigger bounded rebuilding. Diagnostics
show the backend, format, callback sizes/gaps and recovery counts; optional
`output_buffer_frames` preserves the backend default when omitted. RTKit is an
optional Arch host dependency, with guidance shipped in every package.

Validation on the final source and rebuilt packages:

- Formatting and strict Clippy passed; all ordinary Rust targets passed with
  156 tests and 14 ignored. Seven opt-in native audio tests passed, covering
  ALSA null, actual default output and requested 1024/2048-frame buffers.
- Deterministic health tests cover isolated, repeated and persistent XRUN/EIO
  reports, exact error matching, callback startup/stalls, slow periods, mixed
  errors and deadlines that repeated reports cannot extend. Existing reconnect,
  missing-device, cancellation, configuration and gain-preservation tests pass.
- The optimized 0.9.3 binary passed all five native terminal fixtures: quit,
  SIGTERM, connected controller, password settings and token settings.
- The rebuilt Arch package passed installation, integrity, packaged-guide
  identity, startup, terminal/controller fixtures and removal in a container.
- The rebuilt Flatpak passed installed bundle/binary/helper/guide identity,
  configuration isolation, theme access, a disposable real keyring round trip,
  terminal/controller fixtures, callback monitoring on ALSA null, and all four
  null/default/1024/2048 endpoint fixtures in a separate installation.
- Three further silent runs on the physical default output passed. Two
  reproduced the original timing error and reported `timing 1/1 recovered`,
  with 512-frame callbacks continuing at about 11 ms maximum gap. An earlier
  retaining-stream probe independently measured recovery within 10–20 ms.
  These are silent runtime checks, not prolonged audible playback evidence.

The original installed app commit was preserved. Its earlier running instance
had already exited before the final checks; the pre-existing instance set was
unchanged by verification. Logs and package identities are in
`.tools/release-0.9.3-prep/` and
`.tools/flatpak-093-verify/ISOLATED-VERIFICATION.json`.
The standard Flatpak verifier also passed against that isolated installation;
`.tools/flatpak-package/VERIFIED.json` matches the final bundle and binary.

The settings fixtures pass but their HTTP-only test handlers emit background
assertion tracebacks for the event client's `/ws` requests. This is fixture
noise, not a clean validation of the WebSocket path; dedicated event/controller
fixtures cover stream behavior separately.

The precommit candidates established the runtime checks above. GitHub publication
uses the optimized executable built by the main-branch CI run, followed by
package verification of that exact artifact. Final bundling requires a clean
committed tree, records HEAD in BUILDINFO and archives that commit as the source
asset. Native/Arch/Flatpak packages share the verified executable; the Flatpak's
libsecret helper and the packaging wrappers are built locally. No signing or
reproducible-build attestation is implied.

Prolonged audible playback and recovery after a physical-device failure remain
separate checks. Silent callback tests do not establish audible reliability,
acoustic latency or multi-room synchronization.

## 0.9.1 validation (2026-09-16)

A patch release for RUSTSEC-2026-0285 in rustls, which 0.9.0 shipped: the
advisory was published two days before that release and the audit caught it on
the release commit. Formatting, strict Clippy, all test targets, the five
terminal fixtures, the Arch package through install/verify/uninstall in a
disposable container, and the Flatpak through identity, sandbox, keyring, PTY,
controller and silent-audio verification were re-run on a clean checkout of the
release commit. No live server test was repeated for this release; nothing in it
changes how the server is talked to beyond the TLS library.

## 0.9.0 validation (2026-09-16)

Rust formatting, strict Clippy and all ordinary test targets passed (108 tests).
All five native terminal fixture runs passed. The Arch package passed
installation, integrity, desktop and license validation, startup, device
enumeration, PTY/controller fixtures and removal in a disposable Arch container.
The installed Flatpak passed binary/helper identity, demo/device enumeration,
native-config isolation, host theme identity, a real Secret Service round trip,
quit/SIGTERM terminal restoration, the connected controller fixture and the
silent real default-output test.

Three crates entered the graph with no license file of their own —
ratatui-termina, vtparse and wezterm-input-types, all MIT. Copies were taken
from each published crate's own recorded commit, read from its
`.cargo_vcs_info.json`, and their provenance is listed in
`packaging/arch/README.md`. The staging script fails on a dependency with no
license files, which is how they were found.

Live verification during development covered the event stream, podcast browsing
and progress, album art as sixel in foot, and local playback against Music
Assistant 2.10.2 on the development laptop. Acoustic latency, multi-room
synchronization, prolonged playback, broader codec and hardware coverage and
live server-restart recovery were not tested for this release. The sixel encoder
was checked against the specification and by eye in foot, not against other
terminals claiming sixel support.

## First beta validation (2026-09-08)

Rust formatting, strict Clippy and all ordinary test targets passed. All five
native terminal fixture runs passed (quit, SIGTERM, connected music/controls,
password settings and token settings). The Arch package passed installation,
integrity, desktop validation, startup, PTY/controller fixtures and removal in a
disposable Arch container. The installed Flatpak passed binary/helper identity,
demo/device enumeration, native-config isolation, host theme identity, a real
Secret Service round trip, quit/SIGTERM terminal restoration, the connected
controller fixture and the silent real default-output test. Flatpak tests used
synthetic credentials and local servers; live acoustic playback was previously
confirmed in the native application. The audio enumeration emits warnings for
unavailable OSS/direct ALSA outputs; its PulseAudio default stream passed.

## Second beta validation (2026-09-08)

Rust formatting, strict Clippy and all ordinary test targets passed on the
renamed tree. All five native terminal fixture runs passed. The Arch package
passed installation, integrity, desktop and license validation, startup,
PTY/controller fixtures and removal in a disposable Arch container. The
installed Flatpak passed binary/helper identity, demo/device enumeration,
native-config isolation, host theme identity, a real Secret Service round trip,
quit/SIGTERM terminal restoration, the connected controller fixture and the
silent real default-output test.

Two packaging gates were repaired to reach this point, both broken before the
rename. `verify-container.sh` never copied the LICENSE that `PKGBUILD.in` began
listing as a source, so makepkg failed; the script now copies it and checks the
installed copy. The Flatpak targeted Freedesktop Platform 25.08 while this build
host had moved to glibc 2.44, which the runtime's glibc 2.42 cannot satisfy;
the build now targets 26.08 and compares the staged executable's glibc
requirement with the runtime before building.

Live audible playback was not re-tested for this release; the visualizer and
control changes were exercised through fixtures and the offline demo only.
