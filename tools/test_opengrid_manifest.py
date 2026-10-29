#!/usr/bin/env python3
"""Checks that opengrid_manifest.py catches what the QA rule says it must: a class that disagrees with
the minimum system, a minimum that disagrees with the binary, and a missing flag."""
import json, plistlib, shutil, subprocess, sys, tempfile, unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
import opengrid_manifest as m

OLD_BINARY = Path.home() / 'Projects/OpenGridder-Server/legacy-server/target/legacy-server/OpenGridderLegacyServer'  # i386 + x86_64, macOS 10.6


def make_zip(folder, name, stamp):
    work = Path(tempfile.mkdtemp())
    app = work / 'Thing.app' / 'Contents'
    (app / 'MacOS').mkdir(parents=True)
    shutil.copy(OLD_BINARY, app / 'MacOS' / 'thing')
    info = {'CFBundleExecutable': 'thing'}
    info.update(stamp)
    (app / 'Info.plist').write_bytes(plistlib.dumps(info))
    out = Path(folder) / name
    subprocess.run(['ditto', '-c', '-k', '--keepParent', str(work / 'Thing.app'), str(out)], check=True)
    shutil.rmtree(work)


@unittest.skipUnless(OLD_BINARY.exists(), 'needs the 10.6 legacy server build')
class Checks(unittest.TestCase):
    def run_case(self, stamp):
        folder = Path(tempfile.mkdtemp())
        make_zip(folder, 'Thing.zip', stamp)
        manifest = m.build(folder, 'Thing', '1.0')
        problems = m.check(manifest['archives'][0])
        shutil.rmtree(folder)
        return problems

    def test_correct_legacy_build_passes(self):
        self.assertEqual(self.run_case({'OGBuildClass': 'legacy', 'OGMinOS': '10.6'}), [])

    def test_modern_class_with_an_old_minimum_fails(self):
        self.assertTrue(any('does not agree' in p for p in self.run_case({'OGBuildClass': 'modern', 'OGMinOS': '10.6'})))

    def test_minimum_that_the_binary_does_not_have_fails(self):
        self.assertTrue(any('the binaries say' in p for p in self.run_case({'OGBuildClass': 'modern', 'OGMinOS': '11.0'})))

    def test_missing_flag_fails(self):
        self.assertTrue(any('missing' in p for p in self.run_case({})))

    def test_class_boundaries(self):
        self.assertEqual(m.rule_class('macos', '11.0'), 'modern')
        self.assertEqual(m.rule_class('macos', '10.15'), 'legacy')
        self.assertEqual(m.rule_class('windows', '10.0'), 'modern')
        self.assertEqual(m.rule_class('windows', '6.1'), 'legacy')


if __name__ == '__main__':
    unittest.main()
