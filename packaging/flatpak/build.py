#!/usr/bin/env python3
"""Wrap the staged native build in a Flatpak; no host configuration is copied."""
import hashlib
import json
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'packaging'))
from build_input import select_build_input

WORK = ROOT / '.tools/flatpak-package'
STAGE = ROOT / '.tools/arch-package'
APP = 'io.github.brdweb.MaTui'
RUNTIME = 'org.freedesktop.Platform'
BRANCH = '26.08'
# The application's own branch, which is not the runtime's. It was `beta` while
# the releases were; a ref is not upgraded across branches, and nothing was ever
# published under this application ID, so there is nothing to carry over.
BRANCH_NAME = 'stable'
SOURCE_URL = 'https://download.gnome.org/sources/libsecret/0.21/libsecret-0.21.7.tar.xz'
SOURCE_SHA256 = '6b452e4750590a2b5617adc40026f28d2f4903de15f1250e1d1c40bfd68ed55e'
FINISH_ARGS = [
    '--command=ma-tui', '--share=network', '--socket=pulseaudio',
    '--talk-name=org.freedesktop.secrets',
    '--own-name=org.mpris.MediaPlayer2.ma_tui',
    '--talk-name=org.freedesktop.Notifications',
    '--filesystem=~/.local/state/omarchy/current:ro',
]


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def version_tuple(text, pattern):
    return max(tuple(map(int, m)) for m in re.findall(pattern, text))


def check_runtime_compatibility(binary):
    """The selected executable's glibc must not be newer than the runtime's.
    Report that directly instead of leaving a
    missing-symbol failure to the first sandboxed run."""
    needs = version_tuple(run('readelf', '--version-info', str(binary)), r'GLIBC_(\d+)\.(\d+)')
    provides = version_tuple(
        run('flatpak', 'run', '--command=getconf', f'{RUNTIME}//{BRANCH}', 'GNU_LIBC_VERSION'),
        r'glibc (\d+)\.(\d+)')
    assert needs <= provides, (
        f'The staged executable needs glibc {needs[0]}.{needs[1]}, but '
        f'{RUNTIME}//{BRANCH} provides {provides[0]}.{provides[1]}. Build it on a host '
        'whose glibc is no newer than the runtime, or raise BRANCH to a runtime that '
        'matches this host.')


def main():
    build_input = select_build_input(ROOT)
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    assert (STAGE / 'VERSION').read_text().strip() == version
    build_input.check_stage(STAGE)
    assert sha(STAGE / 'LICENSE') == sha(ROOT / 'LICENSE'), 'Staged license is stale'
    check_runtime_compatibility(STAGE / 'ma-tui')
    WORK.mkdir(parents=True, exist_ok=True)
    build = WORK / 'build'
    if build.exists():
        shutil.rmtree(build)
    # Binary-only packaging needs the Platform, not a downloaded compiler SDK.
    run('flatpak', 'build-init', str(build), APP, RUNTIME, RUNTIME, BRANCH)
    files = build / 'files'
    (files / 'bin').mkdir(parents=True, exist_ok=True)
    shutil.copy2(STAGE / 'ma-tui', files / 'bin/ma-tui')
    notices = files / 'share/licenses/ma-tui'
    shutil.copytree(STAGE / 'third-party', notices)
    shutil.copy2(STAGE / 'DEVELOPMENT-STATUS', notices / 'DEVELOPMENT-STATUS')
    shutil.copy2(STAGE / 'LICENSE', notices / 'LICENSE')
    source = WORK / 'libsecret-0.21.7.tar.xz'
    if not source.exists():
        with urllib.request.urlopen(SOURCE_URL, timeout=60) as response:
            source.write_bytes(response.read())
    assert sha(source) == SOURCE_SHA256, 'libsecret source checksum mismatch'
    # Compile the unmodified helper against host headers; use runtime shared libs.
    helper = WORK / 'helper'
    helper.mkdir(exist_ok=True)
    with tarfile.open(source) as archive:
        (helper / 'secret-tool.c').write_bytes(archive.extractfile('libsecret-0.21.7/tool/secret-tool.c').read())
        (notices / 'libsecret-COPYING').write_bytes(archive.extractfile('libsecret-0.21.7/COPYING').read())
    shutil.copy2(source, notices / source.name)
    (helper / 'config.h').write_text('#define GETTEXT_PACKAGE "libsecret"\n#define LOCALEDIR "/app/share/locale"\n')
    flags = shlex.split(run('pkg-config', '--cflags', '--libs', 'libsecret-1'))
    run('cc', '-O2', '-DSECRET_API_SUBJECT_TO_CHANGE', '-DSECRET_COMPILATION', '-I' + str(helper),
        str(helper / 'secret-tool.c'), '-o', str(files / 'bin/secret-tool'), *flags)
    applications = files / 'share/applications'
    applications.mkdir(parents=True)
    shutil.copy2(ROOT / 'packaging/flatpak' / f'{APP}.desktop', applications)
    run('desktop-file-validate', str(applications / f'{APP}.desktop'))
    docs = files / 'share/doc/ma-tui'
    docs.mkdir(parents=True)
    shutil.copy2(ROOT / 'packaging/flatpak/README.md', docs / 'README.md')
    shutil.copy2(ROOT / 'docs/audio-troubleshooting.md', docs / 'audio-troubleshooting.md')
    metadata = {
        **build_input.metadata,
        'version': version, 'app_id': APP, 'branch': BRANCH_NAME,
        'runtime': f'{RUNTIME}/x86_64/{BRANCH}',
        'runtime_commit': run('flatpak', 'info', '--show-commit', f'{RUNTIME}//{BRANCH}'),
        'binary_sha256': sha(files / 'bin/ma-tui'),
        'secret_tool_sha256': sha(files / 'bin/secret-tool'),
        'libsecret_source_sha256': SOURCE_SHA256,
        'compiler': run('cc', '--version').splitlines()[0],
        'finish_args': FINISH_ARGS,
    }
    (docs / 'BUILDINFO.json').write_text(json.dumps(metadata, indent=2) + '\n')
    run('flatpak', 'build-finish', *FINISH_ARGS, str(build))
    assert run('flatpak', 'build', str(build), '/app/bin/ma-tui', '--version') == f'ma-tui {version}'
    run('flatpak', 'build-export', str(WORK / 'repo'), str(build), BRANCH_NAME)
    bundle = WORK / f'ma-tui-v{version}-linux-x86_64.flatpak'
    if bundle.exists():
        bundle.unlink()
    run('flatpak', 'build-bundle', '--runtime-repo=https://flathub.org/repo/flathub.flatpakrepo',
        str(WORK / 'repo'), str(bundle), APP, BRANCH_NAME)
    metadata['bundle_sha256'] = sha(bundle)
    (WORK / 'BUILDINFO.json').write_text(json.dumps(metadata, indent=2) + '\n')
    print(bundle)


if __name__ == '__main__':
    main()
