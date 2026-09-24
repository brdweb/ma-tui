# Omarchy / Arch package

This wraps the tested Linux x86-64 release executable with Arch's real `makepkg`.
It is not a source rebuild on Arch, AUR submission, signed release or deployment.
No install script, service, credentials or personal configuration is included.

## Build and test

Activate the Rust/build environment described in `docs/development-environment.md`
if using the repository-local toolchain. Docker must be available. From repo root:

```sh
cargo build --release --locked
python3 packaging/arch/stage.py
docker run --rm -e BUILD_UID="$(id -u)" \
  -v "$PWD/.tools/arch-package:/work" \
  -v "$PWD/packaging/arch/verify-container.sh:/verify.sh:ro" \
  archlinux:base bash /verify.sh
```

To package a GitHub-built executable, set `MA_TUI_CI_BUILD` to the extracted
`ma-tui-release-build` artifact directory and omit the local Cargo build. The
source commit/tree, version, lockfile and binary hash must match the checkout.
The artifact supplies its own compiler/runtime notices; the local compiler is
not recorded as the origin of a CI-built executable.

`stage.py` copies an explicit allowlist, checks the executable version/architecture,
derives the glibc symbol requirement with readelf, and generates source checksums.
It gathers shipped license/notice files and source links for the locked Linux
Cargo metadata graph (including build dependencies), plus Rust runtime notices.
It fails when any dependency has no license files. Registry sources are unmodified.
Some crates omit their repository-level license files. The supplied copies are
taken from each published crate's own recorded commit, read from the
`.cargo_vcs_info.json` in its registry source, not from a branch:

- dasp_sample 0.11.0 (MIT/Apache-2.0)
  - https://raw.githubusercontent.com/RustAudio/dasp/97c3bb9b2363c0b46ac1633858bf1054fd02a980/LICENSE-MIT
  - https://raw.githubusercontent.com/RustAudio/dasp/97c3bb9b2363c0b46ac1633858bf1054fd02a980/LICENSE-APACHE
- ratatui-termina 0.1.0 (MIT)
  - https://raw.githubusercontent.com/ratatui/ratatui/e665c36cb14752a61cd777fbd06dbef8474f2add/LICENSE
- vtparse 0.6.2 (MIT)
  - https://raw.githubusercontent.com/wezterm/wezterm/edeae72b5fc55c7fa4aa1d08bbefd08c5493f757/LICENSE.md
- wezterm-input-types 0.1.0 (MIT)
  - https://raw.githubusercontent.com/wezterm/wezterm/d5ca5509b95edc31291ba29bd1fcbf3f0b12c92d/LICENSE.md

The container installs build/test dependencies only within the disposable container,
builds as a non-root user, then installs the package with normal dependency and
signature policy checks. It checks integrity, version, snapshot, device enumeration,
interactive quit/SIGTERM cleanup and HTTP-fixture controls, then uninstalls it.
No live MA server or host audio device is mounted or accessed.

Rootless Docker maps the bind-mount owner to container root. Build in a temporary
builder-owned directory inside the container, not directly on the bind mount.
The minimal Arch image excludes documentation via NoExtract; remove that setting
only inside the disposable test container so package integrity covers the guide.
Never change the user's pacman policy to make tests pass.

The successful validation image digest was:
`archlinux:base@sha256:82b1b08faae9d61e3e7e13d562f4d09114d939105b0d59ff34140f3bd418593a`.
The container updated its packages from Arch repositories before building. Pinning
the base image alone does not make that update or the binary build reproducible.
`.BUILDINFO` describes the wrapping environment, not the original Rust compiler
host. `Cargo.lock` and the generated PKGBUILD source checksum identify the inputs.

The result is named in `.tools/arch-package/PACKAGE-NAME`; for this release it is
`ma-tui-1.0.0-1-x86_64.pkg.tar.zst`. Version, glibc requirement and install
instructions are derived from the manifest and built executable. The desktop
launcher is included and validated during package installation.
Copy only a successfully verified package into ignored `dist/`, and create its
checksum using `sha256sum` with a relative package filename. Keep verification logs
in `.tools/`. Do not claim signing, publishing, physical playback or live-server
compatibility based on this packaging test. `INSTALL.txt` is the user-facing guide.

After merging the tested source tree, `python3 packaging/release.py` bundles the
verified package, native binary archive, source and build information in a versioned
dist directory with SHA256SUMS. See `docs/releasing.md` for publication checks.
