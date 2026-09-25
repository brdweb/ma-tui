"""Offline regression checks for CI executable provenance and notice staging."""
import hashlib
import importlib.util
import json
import os
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

REPOSITORY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPOSITORY / 'packaging'))
import build_input


class BuildInputTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'checkout'
        self.root.mkdir()
        (self.root / 'Cargo.toml').write_text('[package]\nversion="0.9.3"\n')
        (self.root / 'Cargo.lock').write_text('fixture lock\n')
        self.artifact = self.root.parent / 'ci-artifact'
        self.artifact.mkdir()
        header = bytearray(64)
        header[:6] = b'\x7fELF\x02\x01'
        header[18:20] = (62).to_bytes(2, 'little')
        self.binary = bytes(header) + b'CI executable'
        (self.artifact / 'ma-tui').write_bytes(self.binary)
        (self.artifact / 'ma-tui').chmod(0o755)
        self.runtime(self.artifact / 'rust-runtime', 'CI')
        self.sysroot = self.root.parent / 'host-rust'
        self.runtime(self.sysroot / 'share/doc/rust', 'HOST')
        self.metadata = {
            'version': '0.9.3', 'commit': '1' * 40, 'source_tree': '2' * 40,
            'target': 'x86_64-unknown-linux-gnu', 'build_machine': 'x86_64',
            'rustc': 'rustc CI compiler', 'cargo': 'cargo CI compiler',
            'cargo_lock_sha256': build_input.sha256(self.root / 'Cargo.lock'),
            'binary_sha256': hashlib.sha256(self.binary).hexdigest(),
            'repository': 'brdweb/ma-tui', 'run_id': '12345', 'run_attempt': 1,
            'run_url': 'https://github.com/brdweb/ma-tui/actions/runs/12345',
        }
        self.write_metadata()
        self.calls = []
        self.addCleanup(patch.stopall)
        patch.dict(os.environ, {'MA_TUI_CI_BUILD': str(self.artifact)}, clear=True).start()
        patch.object(build_input, 'command', side_effect=self.command).start()

    @staticmethod
    def runtime(path, marker):
        (path / 'licenses').mkdir(parents=True)
        (path / 'licenses/LICENSE-MIT').write_text(marker + ' compiler license')
        (path / 'COPYRIGHT-library.html').write_text(marker + ' compiler copyright')

    def write_metadata(self, metadata=None):
        (self.artifact / 'CI-BUILD.json').write_text(json.dumps(
            self.metadata if metadata is None else metadata))

    def command(self, root, *args):
        self.calls.append(args)
        answers = {
            ('git', 'rev-parse', 'HEAD'): '1' * 40,
            ('git', 'rev-parse', 'HEAD^{tree}'): '2' * 40,
            ('rustc', '--print', 'sysroot'): str(self.sysroot),
            ('rustc', '--version'): 'rustc HOST compiler',
            ('cargo', '--version'): 'cargo HOST compiler',
        }
        if len(args) == 2 and args[1] == '--version' and Path(args[0]).name == 'ma-tui':
            return 'ma-tui 0.9.3'
        return answers[args]

    def test_ci_input_preserves_actual_compiler_origin_and_runtime_notices(self):
        selected = build_input.select_build_input(self.root)
        self.assertEqual(selected.binary, self.artifact / 'ma-tui')
        self.assertEqual(selected.rust_runtime, self.artifact / 'rust-runtime')
        self.assertEqual(selected.metadata['rustc'], 'rustc CI compiler')
        self.assertEqual(selected.metadata['cargo'], 'cargo CI compiler')
        self.assertEqual(selected.metadata['build_origin'], {
            'kind': 'github-actions', 'repository': 'brdweb/ma-tui',
            'run_id': '12345', 'run_attempt': 1,
            'run_url': 'https://github.com/brdweb/ma-tui/actions/runs/12345',
        })
        self.assertFalse(any(args[0] in ('rustc', 'cargo') for args in self.calls))

    def test_absent_ci_environment_uses_existing_local_binary_and_host_notices(self):
        os.environ.pop('MA_TUI_CI_BUILD')
        local = self.root / 'target/release/ma-tui'
        local.parent.mkdir(parents=True)
        local.write_bytes(self.binary + b' local')
        local.chmod(0o755)
        selected = build_input.select_build_input(self.root)
        self.assertEqual(selected.binary, local)
        self.assertEqual(selected.rust_runtime, self.sysroot / 'share/doc/rust')
        self.assertEqual(selected.metadata['build_origin'], {'kind': 'local'})
        self.assertEqual(selected.metadata['rustc'], 'rustc HOST compiler')
        self.assertEqual(selected.metadata['binary_sha256'], build_input.sha256(local))

    def test_mismatched_source_version_target_and_hash_are_rejected(self):
        for field, value in [
            ('commit', '3' * 40), ('source_tree', '4' * 40),
            ('version', '0.9.2'), ('target', 'aarch64-unknown-linux-gnu'),
            ('build_machine', 'aarch64'),
            ('cargo_lock_sha256', '0' * 64), ('binary_sha256', '0' * 64),
        ]:
            with self.subTest(field=field):
                self.write_metadata({**self.metadata, field: value})
                with self.assertRaises(ValueError):
                    build_input.select_build_input(self.root)

    def test_invalid_json_shapes_types_and_provenance_are_rejected(self):
        examples = [[], True, 'not an object', {},
                    {**self.metadata, 'binary_path': '/elsewhere/binary'}]
        for field, value in [
            ('version', 0.93), ('commit', 1), ('rustc', []), ('cargo', None),
            ('run_id', True), ('run_id', 1.5), ('run_attempt', 0),
            ('repository', '../wrong/repo'), ('run_url', 'https://example.com/12345'),
        ]:
            examples.append({**self.metadata, field: value})
        for metadata in examples:
            with self.subTest(metadata=metadata):
                self.write_metadata(metadata)
                with self.assertRaises(ValueError):
                    build_input.select_build_input(self.root)
        (self.artifact / 'CI-BUILD.json').write_text('{broken json')
        with self.assertRaises(ValueError):
            build_input.select_build_input(self.root)

    def test_empty_explicit_ci_path_never_falls_back_to_local(self):
        os.environ['MA_TUI_CI_BUILD'] = ' '
        with self.assertRaisesRegex(ValueError, 'MA_TUI_CI_BUILD'):
            build_input.select_build_input(self.root)

    def test_wrong_elf_architecture_is_rejected_even_with_matching_hash(self):
        wrong = bytearray(self.binary)
        wrong[18:20] = (183).to_bytes(2, 'little')
        (self.artifact / 'ma-tui').write_bytes(wrong)
        self.metadata['binary_sha256'] = hashlib.sha256(wrong).hexdigest()
        self.write_metadata()
        with self.assertRaisesRegex(ValueError, 'x86-64 ELF'):
            build_input.select_build_input(self.root)

    def test_binary_reported_version_must_match_manifest(self):
        real_command = self.command
        def wrong_version(root, *args):
            if args == (str(self.artifact / 'ma-tui'), '--version'):
                return 'ma-tui 0.9.2'
            return real_command(root, *args)
        with patch.object(build_input, 'command', side_effect=wrong_version):
            with self.assertRaisesRegex(ValueError, 'version'):
                build_input.select_build_input(self.root)

    def test_missing_executable_mode_fails_without_changing_the_input(self):
        binary = self.artifact / 'ma-tui'
        binary.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'file modes preserved'):
            build_input.select_build_input(self.root)
        self.assertEqual(binary.stat().st_mode & 0o777, 0o644)

    def test_missing_and_symlinked_runtime_notices_are_rejected(self):
        license_path = self.artifact / 'rust-runtime/licenses/LICENSE-MIT'
        license_path.unlink()
        with self.assertRaisesRegex(ValueError, 'empty'):
            build_input.select_build_input(self.root)
        license_path.symlink_to(self.sysroot / 'share/doc/rust/licenses/LICENSE-MIT')
        with self.assertRaisesRegex(ValueError, 'unsafe'):
            build_input.select_build_input(self.root)

    def test_stage_identity_checks_both_binary_and_provenance(self):
        selected = build_input.select_build_input(self.root)
        stage = self.root / 'stage'
        stage.mkdir()
        (stage / 'ma-tui').write_bytes(self.binary)
        metadata_file = stage / 'BUILD-INPUT.json'
        metadata_file.write_text(json.dumps(selected.metadata))
        selected.check_stage(stage)
        metadata_file.write_text(json.dumps({**selected.metadata, 'rustc': 'wrong compiler'}))
        with self.assertRaisesRegex(ValueError, 'provenance'):
            selected.check_stage(stage)
        metadata_file.write_text(json.dumps(selected.metadata))
        (stage / 'ma-tui').write_bytes(self.binary + b' wrong')
        with self.assertRaisesRegex(ValueError, 'executable'):
            selected.check_stage(stage)

    def test_arch_staging_uses_ci_binary_and_ci_rust_notices(self):
        spec = importlib.util.spec_from_file_location(
            'fixture_arch_stage', REPOSITORY / 'packaging/arch/stage.py')
        stage_module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(stage_module)
        for name in ['README.md', 'LICENSE', 'packaging/ma-tui.desktop',
                     'packaging/ma-tui.svg', 'docs/audio-troubleshooting.md',
                     'packaging/arch/DEVELOPMENT-STATUS',
                     'packaging/arch/INSTALL.txt', 'packaging/arch/PKGBUILD.in',
                     'tests/terminal_smoke.py', 'tests/connected_smoke.py']:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture ' + name)
        crate = self.root / 'registry/sendspin-0.3.7'
        crate.mkdir(parents=True)
        (crate / 'LICENSE').write_text('dependency license')
        packages = {'packages': [{
            'source': 'registry+fixture', 'name': 'sendspin', 'version': '0.3.7',
            'manifest_path': str(crate / 'Cargo.toml'), 'license': 'MIT',
            'repository': 'https://example.invalid/sendspin',
        }]}
        def stage_command(*args):
            if args[0] == 'cargo':
                return json.dumps(packages)
            if args[:2] == ('readelf', '-h'):
                return 'Machine: Advanced Micro Devices X86-64'
            if args[:2] == ('readelf', '--version-info'):
                return 'GLIBC_2.39'
            return self.command(self.root, *args)
        stage = self.root / 'arch-stage'
        with patch.object(stage_module, 'ROOT', self.root), \
                patch.object(stage_module, 'STAGE', stage), \
                patch.object(stage_module, 'command', side_effect=stage_command):
            stage_module.stage()
        self.assertEqual((stage / 'ma-tui').read_bytes(), self.binary)
        self.assertEqual((stage / 'third-party/rust-runtime/licenses/LICENSE-MIT').read_text(),
                         'CI compiler license')
        self.assertEqual((stage / 'third-party/rust-runtime/COPYRIGHT-library.html').read_text(),
                         'CI compiler copyright')
        build_input.select_build_input(self.root).check_stage(stage)

    def prepare_release(self):
        spec = importlib.util.spec_from_file_location(
            'fixture_release', REPOSITORY / 'packaging/release.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        stage = self.root / '.tools/arch-package'
        stage.mkdir(parents=True)
        for name in ['README.md', 'LICENSE', 'docs/audio-troubleshooting.md']:
            source = self.root / name
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text('fixture ' + name)
            (stage / source.name).write_bytes(source.read_bytes())
        for name in ['ma-tui.desktop', 'ma-tui.svg', 'INSTALL.txt', 'DEVELOPMENT-STATUS']:
            (stage / name).write_text('fixture ' + name)
        (stage / 'third-party').mkdir()
        (stage / 'ma-tui').write_bytes(self.binary)
        (stage / 'VERSION').write_text('0.9.3\n')
        (stage / 'PACKAGE-NAME').write_text('ma-tui-0.9.3-1-x86_64.pkg.tar.zst\n')
        (stage / 'ma-tui-0.9.3-1-x86_64.pkg.tar.zst').write_bytes(b'fixture Arch archive')
        selected = build_input.select_build_input(self.root)
        (stage / 'BUILD-INPUT.json').write_text(json.dumps(selected.metadata))
        flatpak = self.root / '.tools/flatpak-package'
        flatpak.mkdir()
        bundle = flatpak / 'ma-tui-v0.9.3-linux-x86_64.flatpak'
        bundle.write_bytes(b'fixture Flatpak bundle')
        flatpak_info = {**selected.metadata, 'bundle_sha256': build_input.sha256(bundle)}
        (flatpak / 'BUILDINFO.json').write_text(json.dumps(flatpak_info))
        (flatpak / 'VERIFIED.json').write_text(json.dumps({
            key: flatpak_info[key] for key in ('bundle_sha256', 'binary_sha256')}))
        patch.object(module, 'ROOT', self.root).start()
        patch.object(module, 'STAGE', stage).start()
        return module, flatpak

    def test_release_archive_records_ci_compilers_origin_and_exact_executable(self):
        release, _ = self.prepare_release()
        def release_command(*args):
            if args == ('git', 'status', '--porcelain'):
                return ''
            if args[:2] == ('readelf', '--version-info'):
                return 'GLIBC_2.39'
            raise AssertionError(args)
        def archive_source(args, **kwargs):
            self.assertEqual(args[:3], ['git', 'archive', '--format=tar.gz'])
            Path(args[args.index('-o') + 1]).write_bytes(b'fixture source archive')
        with patch.object(release, 'command', side_effect=release_command), \
                patch.object(release.subprocess, 'check_output', return_value=self.binary), \
                patch.object(release.subprocess, 'run', side_effect=archive_source):
            release.main()
        destination = self.root / 'dist/v0.9.3'
        manifest = json.loads((destination / 'BUILDINFO.json').read_text())
        self.assertEqual(manifest['rustc'], 'rustc CI compiler')
        self.assertEqual(manifest['cargo'], 'cargo CI compiler')
        self.assertEqual(manifest['build_origin']['kind'], 'github-actions')
        self.assertEqual(manifest['build_origin']['run_id'], '12345')
        with tarfile.open(destination / 'ma-tui-v0.9.3-linux-x86_64.tar.gz') as archive:
            self.assertEqual(archive.extractfile('ma-tui-v0.9.3/ma-tui').read(), self.binary)
        self.assertTrue((destination / 'SHA256SUMS').is_file())

    def test_release_still_rejects_dirty_checkout_and_unverified_final_bundle(self):
        release, flatpak = self.prepare_release()
        with patch.object(release, 'command', return_value=' M source.rs'):
            with self.assertRaisesRegex(AssertionError, 'Commit release changes first'):
                release.main()
        verified = json.loads((flatpak / 'VERIFIED.json').read_text())
        verified['bundle_sha256'] = '0' * 64
        (flatpak / 'VERIFIED.json').write_text(json.dumps(verified))
        with patch.object(release, 'command', return_value=''), \
                patch.object(release.subprocess, 'check_output', return_value=self.binary):
            with self.assertRaises(AssertionError):
                release.main()
        self.assertFalse((self.root / 'dist').exists())


if __name__ == '__main__':
    unittest.main()
