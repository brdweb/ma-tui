#!/usr/bin/env python3
"""Stage a binary package; makepkg performs the actual Arch packaging.

Run from any directory with Cargo/Rust tools available. Only an explicit allowlist
of build outputs/docs and registry license files is copied, never local config.
"""
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'packaging'))
from build_input import select_build_input

STAGE = ROOT / '.tools/arch-package'


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True)


def stage():
    build_input = select_build_input(ROOT)
    binary = build_input.binary
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    assert re.fullmatch(r'\d+\.\d+\.\d+(?:-beta\.\d+)?', version), version
    pkgver = version.replace('-', '')
    package = f'ma-tui-{pkgver}-1-x86_64.pkg.tar.zst'
    assert command(str(binary), '--version').strip() == f'ma-tui {version}'
    assert 'Advanced Micro Devices X86-64' in command('readelf', '-h', str(binary))
    versions = re.findall(r'GLIBC_(\d+)\.(\d+)', command('readelf', '--version-info', str(binary)))
    glibc = '.'.join(map(str, max(tuple(map(int, v)) for v in versions)))
    metadata = json.loads(command('cargo', 'metadata', '--offline', '--locked',
                                  '--format-version', '1', '--filter-platform',
                                  'x86_64-unknown-linux-gnu'))
    packages = sorted((p for p in metadata['packages'] if p['source']),
                      key=lambda p: (p['name'], p['version']))
    assert any(p['name'] == 'sendspin' and p['version'] == '0.3.7' for p in packages)
    STAGE.mkdir(parents=True, exist_ok=True)
    notices = STAGE / 'third-party'
    if notices.exists():
        shutil.rmtree(notices)
    notices.mkdir()
    inventory = []
    for p in packages:
        base = Path(p['manifest_path']).parent
        files = [f for f in base.rglob('*') if f.is_file() and
                 f.name.lower().startswith(('license', 'licence', 'copying', 'notice', 'copyright'))]
        if not files:
            base = ROOT / 'packaging/arch/licenses' / f"{p['name']}-{p['version']}"
            files = list(base.glob('*'))
        if not files:
            raise RuntimeError(f"Missing license files: {p['name']} {p['version']}")
        for f in files:
            assert not f.is_symlink(), f
            dest = notices / f"{p['name']}-{p['version']}" / f.relative_to(base)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(f, dest)
        inventory.append({'name': p['name'], 'version': p['version'],
                          'declared_license': p['license'], 'repository': p['repository'],
                          'unmodified_source': f"https://crates.io/api/v1/crates/{p['name']}/{p['version']}/download"})
    rust_docs = build_input.rust_runtime
    shutil.copytree(rust_docs / 'licenses', notices / 'rust-runtime/licenses')
    shutil.copyfile(rust_docs / 'COPYRIGHT-library.html', notices / 'rust-runtime/COPYRIGHT-library.html')
    (notices / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
    with tarfile.open(STAGE / 'THIRD-PARTY-NOTICES.tar.gz', 'w:gz') as archive:
        archive.add(notices, arcname='third-party')
    for source, name in [(binary, 'ma-tui'), (ROOT / 'README.md', 'README.md'),
                         (ROOT / 'docs/audio-troubleshooting.md', 'audio-troubleshooting.md'),
                         (ROOT / 'LICENSE', 'LICENSE'),
                         (ROOT / 'packaging/ma-tui.desktop', 'ma-tui.desktop'),
                         (ROOT / 'packaging/ma-tui.svg', 'ma-tui.svg'),
                         (ROOT / 'packaging/arch/DEVELOPMENT-STATUS', 'DEVELOPMENT-STATUS')]:
        shutil.copyfile(source, STAGE / name)
    install = (ROOT / 'packaging/arch/INSTALL.txt').read_text()
    (STAGE / 'INSTALL.txt').write_text(install.replace('@VERSION@', version).replace('@PACKAGE@', package).replace('@GLIBC@', glibc))
    (STAGE / 'PACKAGE-NAME').write_text(package + '\n')
    (STAGE / 'VERSION').write_text(version + '\n')
    (STAGE / 'BUILD-INPUT.json').write_text(json.dumps(build_input.metadata, indent=2) + '\n')
    (STAGE / 'ma-tui').chmod(0o755)
    sources = ['ma-tui', 'ma-tui.desktop', 'ma-tui.svg', 'README.md', 'audio-troubleshooting.md', 'INSTALL.txt', 'THIRD-PARTY-NOTICES.tar.gz', 'DEVELOPMENT-STATUS', 'LICENSE']
    sums = ' '.join("'" + hashlib.sha256((STAGE / name).read_bytes()).hexdigest() + "'" for name in sources)
    template = (ROOT / 'packaging/arch/PKGBUILD.in').read_text()
    (STAGE / 'PKGBUILD').write_text(template.replace('@GLIBC@', glibc).replace('@PKGVER@', pkgver).replace('@SHA256SUMS@', sums))
    for name in ['terminal_smoke.py', 'connected_smoke.py']:
        shutil.copyfile(ROOT / 'tests' / name, STAGE / name)
    print(f'Staged {len(inventory)} dependency notice sets; glibc >= {glibc}; {STAGE}')


if __name__ == '__main__':
    stage()
