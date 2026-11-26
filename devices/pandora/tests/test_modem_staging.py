#!/usr/bin/env python3
"""Linux-only isolated tests of start_modem.sh; no real mount or device access.

Run: python3 devices/pandora/tests/test_modem_staging.py
Only the helper's absolute paths are redirected into a TemporaryDirectory.
getprop/readlink/mount/umount are replaced with local fixtures. These tests
establish shell control flow, not kernel mount semantics or modem readiness.
"""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

SOURCE = (Path(__file__).resolve().parents[1] / 'system/bin/start_modem.sh').read_text(encoding='utf-8')
MOCK = r'''
import os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
root = pathlib.Path(os.environ['CASE_ROOT'])
if name == 'getprop':
    print(os.environ['RECOVERY'] if args[0] == 'init.svc.recovery' else os.environ['SLOT'])
elif name == 'readlink':
    print(os.environ['REMOTE_IDENTITY'])
elif name == 'mount':
    target = pathlib.Path(args[-1])
    with (root / 'operations').open('a') as f:
        f.write('mount ' + ' '.join(args) + '\n')
    if os.environ.get('FAIL_TARGET') == str(target) or os.environ.get('FAIL_MOUNT') == '1':
        sys.exit(1)
    if '--bind' in args:
        target.rmdir()
        target.symlink_to(args[-2], target_is_directory=True)
    else:
        (target / 'image/modem_oem').mkdir(parents=True)
        (target / 'image/modem.mdt').write_text('mdt')
        if not os.environ.get('MISSING_OEM'):
            (target / 'image/modem_oem/qdsp6_oemsw.mbn').write_text('oem')
elif name == 'umount':
    target = pathlib.Path(args[0])
    with (root / 'operations').open('a') as f:
        f.write('umount ' + str(target) + '\n')
    if target.is_symlink():
        target.unlink()
        target.mkdir()
    else:
        shutil.rmtree(target / 'image')
'''


class ModemStagingTests(unittest.TestCase):
    def run_case(self, name, mode='--stage-only', slot='_b', **overrides):
        with tempfile.TemporaryDirectory(prefix='modem-stage-test-') as td:
            root = Path(td)
            originals = ['/sys/class/remoteproc/remoteproc2', '/proc/mounts', '/mnt',
                         '/vendor/modem_firmware', '/etc/firmware',
                         '/vendor/rfs/msm/mpss/readonly/firmware']
            paths = {p: str(root / p.lstrip('/')) for p in originals}
            (root / 'sys/class/remoteproc/remoteproc2').mkdir(parents=True)
            state = root / 'sys/class/remoteproc/remoteproc2/state'
            initial_state = 'running' if name == 'already_running' else 'offline'
            state.write_text(initial_state)
            (root / 'proc').mkdir()
            (root / 'proc/mounts').write_text('')
            (root / 'mnt').mkdir()
            for p in ['vendor/modem_firmware', 'etc/firmware', 'vendor/rfs/msm/mpss/readonly']:
                (root / p).mkdir(parents=True, exist_ok=True)
            (root / 'vendor/rfs/msm/mpss/readonly/firmware').symlink_to(
                root / 'vendor/modem_firmware', target_is_directory=True)
            pattern = '|'.join(re.escape(p) for p in sorted(paths, key=len, reverse=True))
            helper = root / 'helper.sh'
            helper.write_text(re.sub(pattern, lambda m: paths[m.group()], SOURCE))
            bindir = root / 'bin'
            bindir.mkdir()
            mock = bindir / 'mock'
            mock.write_text('#!' + sys.executable + '\n' + MOCK)
            mock.chmod(0o755)
            for cmd in ['getprop', 'readlink', 'mount', 'umount']:
                (bindir / cmd).symlink_to(mock)
            env = dict(os.environ, CASE_ROOT=str(root), PATH=str(bindir) + ':' + os.environ['PATH'],
                       RECOVERY='running', SLOT=slot,
                       REMOTE_IDENTITY='/sys/devices/platform/soc/4080000.remoteproc-mss/remoteproc/remoteproc2')
            for key in ['FAIL_TARGET', 'FAIL_MOUNT', 'MISSING_OEM']:
                env.pop(key, None)
            env.update(overrides)
            if name == 'canonical_bind_failure':
                env['FAIL_TARGET'] = str(root / 'vendor/modem_firmware')
            if name == 'image_bind_failure':
                env['FAIL_TARGET'] = str(root / 'etc/firmware')
            if name == 'existing_mount':
                (root / 'proc/mounts').write_text('source ' + str(root / 'vendor/modem_firmware') + ' vfat ro 0 0\n')
            proc = subprocess.run(['/bin/sh', str(helper), mode], env=env,
                                  text=True, capture_output=True, timeout=10)
            success = name in ['stage_only', 'start']
            self.assertEqual(proc.returncode == 0, success, (name, proc.stderr))
            self.assertEqual(state.read_text(), 'start\n' if name == 'start' else initial_state)
            operations = root / 'operations'
            ops = operations.read_text() if operations.exists() else ''
            if success:
                self.assertIn('mount -t vfat -o ro /dev/block/by-name/modemfirmware' + slot, ops)
                if name == 'stage_only':
                    self.assertEqual(state.read_text(), 'offline')
                    self.assertIn('umount ', ops)
                    self.assertFalse((root / 'vendor/modem_firmware').is_symlink(), ops)
                    self.assertFalse((root / 'etc/firmware').is_symlink(), ops)
                    self.assertEqual(list((root / 'mnt').iterdir()), [])
                else:
                    self.assertEqual((root / 'vendor/rfs/msm/mpss/readonly/firmware/image/modem_oem/qdsp6_oemsw.mbn').read_text(), 'oem')
                    self.assertEqual((root / 'etc/firmware/modem.mdt').read_text(), 'mdt')
            else:
                self.assertEqual(list((root / 'mnt').iterdir()), [], ops)
                self.assertFalse((root / 'vendor/modem_firmware').is_symlink(), ops)
                self.assertFalse((root / 'etc/firmware').is_symlink(), ops)
            if name in ['existing_mount', 'already_running', 'wrong_identity', 'wrong_slot', 'wrong_mode', 'not_recovery']:
                self.assertEqual(ops, '')

    def test_control_flow(self):
        cases = [
            ('stage_only', {}),
            ('stage_only', {'slot': '_a'}),
            ('start', {'mode': 'start'}),
            ('canonical_bind_failure', {}),
            ('image_bind_failure', {}),
            ('mount_failure', {'FAIL_MOUNT': '1'}),
            ('missing_oem', {'MISSING_OEM': '1'}),
            ('existing_mount', {}),
            ('wrong_mode', {'mode': 'invalid'}),
            ('wrong_slot', {'slot': '_invalid'}),
            ('not_recovery', {'RECOVERY': 'stopped'}),
            ('already_running', {}),
            ('wrong_identity', {'REMOTE_IDENTITY': '/sys/devices/other/remoteproc2'}),
        ]
        for name, options in cases:
            with self.subTest(case=name, options=options):
                self.run_case(name, **options)


if __name__ == '__main__':
    unittest.main(verbosity=2)
