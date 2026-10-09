#!/bin/bash
# Disposable Ubuntu 24.04 only: /source is read-only; /output holds artifacts.
set -euo pipefail

if [[ "${1:-}" == --session ]]; then
    [[ $EUID != 0 && -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]]
    cd "$HOME/matui"
    printf '%s\n' 'ma-tui-disposable-fixture' |
        gnome-keyring-daemon --unlock --components=secrets
    pulseaudio --daemonize=yes --exit-idle-time=-1 --log-target=stderr \
        --load='module-null-sink sink_name=ma_tui_fixture'
    pactl set-default-sink ma_tui_fixture
    pactl info

    flatpak remote-add --user --if-not-exists flathub \
        https://flathub.org/repo/flathub.flatpakrepo
    flatpak install --user --noninteractive flathub org.freedesktop.Platform//26.08
    python3 packaging/flatpak/build.py
    version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')
    flatpak install --user --noninteractive \
        ".tools/flatpak-package/ma-tui-v${version}-linux-x86_64.flatpak"
    flatpak run io.github.brdweb.MaTui --version
    flatpak run io.github.brdweb.MaTui --demo --snapshot
    flatpak run io.github.brdweb.MaTui --list-devices
    python3 packaging/flatpak/verify.py
    exit 0
fi

[[ $EUID == 0 && -f /etc/debian_version && -d /source && -d /output ]]
[[ "${BUILD_UID:-0}" =~ ^[1-9][0-9]*$ ]]
# Require the qualified candidate and its matching GNU Rust toolchain, not Nix builds.
[[ -f /source/.tools/arch-package/BUILD-INPUT.json ]]
[[ -x /source/.tools/rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc ]]
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
    flatpak build-essential pkg-config libsecret-1-dev desktop-file-utils \
    python3 python3-venv cargo git binutils ca-certificates libasound2-dev \
    dbus-daemon gnome-keyring pulseaudio pulseaudio-utils
python3 -m venv /opt/ma-tui-verifier
/opt/ma-tui-verifier/bin/pip install uv
useradd -m -u "$BUILD_UID" builder
workspace=/home/builder/matui
mkdir -p "$workspace/.tools" "$workspace/target/release" /home/builder/.cargo
# Rootless Docker maps the bind owner to root. Copy allowlisted repository/build
# inputs into a builder-owned workspace; never mount the user's home or config.
cp -a /source/.git /source/Cargo.toml /source/Cargo.lock /source/README.md \
    /source/LICENSE /source/src /source/tests /source/packaging /source/docs "$workspace/"
cp -a /source/.tools/arch-package "$workspace/.tools/"
cp /source/target/release/ma-tui "$workspace/target/release/ma-tui"
cp -a /source/.tools/cargo/registry /home/builder/.cargo/
mkdir -m 700 /home/builder/runtime
chown -R builder:builder /home/builder
mkdir -p /run/dbus
dbus-daemon --system --fork
runuser -u builder -- env \
    PATH="/source/.tools/rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:/opt/ma-tui-verifier/bin:$PATH" \
    CARGO_HOME=/home/builder/.cargo \
    XDG_RUNTIME_DIR=/home/builder/runtime \
    dbus-run-session -- bash "$workspace/packaging/flatpak/verify-container.sh" --session
# Export only after every verification assertion passes, leaving failures private.
version=$(python3 -c "import tomllib; print(tomllib.load(open('$workspace/Cargo.toml', 'rb'))['package']['version'])")
install -m644 "$workspace/.tools/flatpak-package/BUILDINFO.json" \
    "$workspace/.tools/flatpak-package/VERIFIED.json" \
    "$workspace/.tools/flatpak-package/ma-tui-v${version}-linux-x86_64.flatpak" /output/
printf 'FLATPAK CONTAINER VERIFIED: artifacts exported; audio used a disposable null sink\n'
