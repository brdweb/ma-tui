#!/usr/bin/env python3
"""Verify an installed bundle; uses only synthetic credentials and local fixtures."""
import hashlib
import json
import os
import stat
import subprocess
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / '.tools/flatpak-package'
APP = 'io.github.brdweb.MaTui'


def run(*args, **kwargs):
    return subprocess.check_output(args, cwd=ROOT, text=True, **kwargs).strip()


def sandbox(*args, **kwargs):
    return run('flatpak', 'run', '--user', '--command=' + args[0], APP, *args[1:], **kwargs)


def main():
    info = json.loads((WORK / 'BUILDINFO.json').read_text())
    bundle = WORK / f"ma-tui-v{info['version']}-linux-x86_64.flatpak"
    assert hashlib.sha256(bundle.read_bytes()).hexdigest() == info['bundle_sha256']
    assert sandbox('sha256sum', '/app/bin/ma-tui').split()[0] == info['binary_sha256']
    assert sandbox('sha256sum', '/app/bin/secret-tool').split()[0] == info['secret_tool_sha256']
    assert sandbox('ma-tui', '--version') == f"ma-tui {info['version']}"
    sandbox('test', '-s', '/app/share/icons/hicolor/scalable/apps/ma-tui.svg')
    assert sandbox('sha256sum', '/app/share/doc/ma-tui/audio-troubleshooting.md').split()[0] == hashlib.sha256((ROOT / 'docs/audio-troubleshooting.md').read_bytes()).hexdigest()
    assert 'MA-TUI' in sandbox('ma-tui', '--demo', '--snapshot')
    assert 'default' in sandbox('ma-tui', '--list-devices').lower()
    sandbox('sh', '-c', 'test ! -e "$HOME/.config/ma-tui/config.toml"')
    sandbox('sh', '-c', 'test -z "${NO_COLOR+x}"')
    theme = Path.home() / '.local/state/omarchy/current/theme/colors.toml'
    if theme.is_file():
        assert sandbox('sha256sum', str(theme)).split()[0] == hashlib.sha256(theme.read_bytes()).hexdigest()
    # Only our uniquely named disposable item is created/read/deleted.
    item = 'ma-tui-flatpak-fixture-' + uuid.uuid4().hex
    value = uuid.uuid4().hex
    try:
        sandbox('secret-tool', 'store', '--label=MA-TUI Flatpak disposable test',
                'application', item, input=value + '\n')
        assert sandbox('secret-tool', 'lookup', 'application', item) == value
    finally:
        sandbox('secret-tool', 'clear', 'application', item)
    with tempfile.TemporaryDirectory(prefix='ma-tui-flatpak-verify-') as tmp:
        wrapper = Path(tmp) / 'ma-tui-flatpak'
        # The connected fixture's temporary TOML needs a read-only test grant.
        # Production metadata never grants /tmp or the host configuration.
        wrapper.write_text('#!/bin/bash\nset -e\nextra=()\nif [[ ${1:-} == --config ]]; then extra+=("--filesystem=$(dirname "$2"):ro"); fi\nexec flatpak run --user "${extra[@]}" io.github.brdweb.MaTui "$@"\n')
        wrapper.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR)
        for script, extra in [('terminal_smoke.py', []), ('terminal_smoke.py', ['sigterm']),
                              ('connected_smoke.py', [])]:
            subprocess.run(['uv', 'run', '--with', 'pyte', 'python', 'tests/' + script,
                            str(wrapper), *extra], cwd=ROOT, check=True,
                           env=dict(os.environ, MA_TUI_TEST_FLATPAK_APP=APP))
    # Existing real CPAL test connects to an in-process fake audio server. It
    # opens the desktop output and acknowledges commands without audio frames.
    result = run('cargo', 'test', '--test', 'audio_null', '--no-run', '--locked', '--message-format=json')
    # Select the test harness itself: cargo also reports the application binary.
    artifacts = [json.loads(line) for line in result.splitlines() if line.startswith('{')]
    binary = next(Path(a['executable']) for a in artifacts
                  if a.get('reason') == 'compiler-artifact' and a.get('executable')
                  and a['profile']['test'] and a['target']['name'] == 'audio_null')
    print(run('flatpak', 'run', '--user', '--filesystem=' + str(binary) + ':ro',
              '--command=' + str(binary), APP,
              'opens_default_output_silently_and_acknowledges_volume', '--ignored', '--exact'))
    (WORK / 'VERIFIED.json').write_text(json.dumps({
        'bundle_sha256': info['bundle_sha256'], 'binary_sha256': info['binary_sha256'],
        'checks': ['installed binary/helper identity', 'version/demo/devices',
                   'native config isolation', 'available Omarchy theme identity',
                   'real keyring round trip', 'PTY quit and SIGTERM',
                   'local HTTP music/control fixture', 'silent real default audio output'],
    }, indent=2) + '\n')
    print('FLATPAK VERIFIED: installed identity, sandbox, keyring, PTY, controller, silent audio')


if __name__ == '__main__':
    main()
