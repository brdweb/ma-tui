#!/bin/bash
# Run only inside a disposable Arch container with staging mounted at /work.
set -euo pipefail
[[ -e /etc/arch-release && -d /work && "${BUILD_UID:-0}" != 0 ]]
pacman-key --init
pacman -Syu --noconfirm --needed fakeroot binutils alsa-lib gcc-libs ca-certificates libsecret desktop-file-utils python python-pyte
useradd -m -u "$BUILD_UID" builder
# Rootless Docker maps the bind mount owner to container root. Build in a
# disposable builder-owned directory, then copy just the package back as root.
mkdir -p /tmp/ma-tui-build
cp /work/PKGBUILD /work/ma-tui /work/ma-tui.desktop /work/ma-tui.svg /work/README.md /work/audio-troubleshooting.md /work/INSTALL.txt /work/THIRD-PARTY-NOTICES.tar.gz /work/DEVELOPMENT-STATUS /work/LICENSE /tmp/ma-tui-build/
chown -R builder:builder /tmp/ma-tui-build
cd /tmp/ma-tui-build
runuser -u builder -- makepkg --noconfirm --force
package_name=$(cat /work/PACKAGE-NAME)
[[ "$package_name" =~ ^ma-tui-[0-9][a-z0-9.]*-1-x86_64.pkg.tar.zst$ ]]
install -m644 "$package_name" /work/
package=/work/$package_name
pacman -Qip "$package"
# The minimal image excludes documentation. Enable it only in this disposable
# container so the package integrity check covers the shipped user guide too.
python -c 'from pathlib import Path; p=Path("/etc/pacman.conf"); p.write_text("\n".join(line for line in p.read_text().splitlines() if not line.lstrip().startswith("NoExtract"))+"\n")'
pacman -U --noconfirm "$package"
pacman -Qkk ma-tui
ma-tui --version
[[ "$(ma-tui --version)" == "ma-tui $(cat /work/VERSION)" ]]
desktop-file-validate /usr/share/applications/ma-tui.desktop
test -s /usr/share/icons/hicolor/scalable/apps/ma-tui.svg
test -s /usr/share/licenses/ma-tui/LICENSE
python -c 'from pathlib import Path; assert Path("/work/audio-troubleshooting.md").read_bytes() == Path("/usr/share/doc/ma-tui/docs/audio-troubleshooting.md").read_bytes()'
ma-tui --demo --snapshot
ma-tui --list-devices
runuser -u builder -- python /work/terminal_smoke.py /usr/bin/ma-tui
runuser -u builder -- python /work/terminal_smoke.py /usr/bin/ma-tui sigterm
runuser -u builder -- python /work/connected_smoke.py /usr/bin/ma-tui
pacman -R --noconfirm ma-tui
test ! -e /usr/bin/ma-tui
test ! -e /usr/share/applications/ma-tui.desktop
test ! -e /usr/share/icons/hicolor/scalable/apps/ma-tui.svg
printf 'ARCH PACKAGE VERIFIED: install, integrity, startup, PTY, HTTP fixtures, uninstall\n'
