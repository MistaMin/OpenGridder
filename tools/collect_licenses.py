#!/usr/bin/env python3
"""Collect native release dependency notices without copying SDK sources/docs."""
import argparse
import json
import shutil
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--aax-sdk', type=Path, required=True)
parser.add_argument('--vst3-sdk', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
shutil.copy2(root / 'LICENSE', out / 'OpenGridder-LICENSE.txt')
shutil.copy2(root / 'THIRD_PARTY_LICENSES.md', out / 'THIRD_PARTY_LICENSES.md')
for name, sdk in [('AAX-SDK', args.aax_sdk), ('VST3-SDK', args.vst3_sdk)]:
    target = out / 'third-party' / name
    target.mkdir(parents=True, exist_ok=True)
    shutil.copy2(sdk / 'LICENSE.txt', target / 'LICENSE.txt')
metadata = json.loads(subprocess.check_output([
    'cargo', 'metadata', '--locked', '--offline', '--filter-platform',
    'aarch64-apple-darwin', '--format-version', '1'], cwd=root))
packages = {p['id']: p for p in metadata['packages']}
nodes = {n['id']: n for n in metadata['resolve']['nodes']}
pending = [p['id'] for p in packages.values() if p['name'] in ('og-plugin', 'og-server')]
seen = set()
while pending:
    key = pending.pop()
    if key in seen:
        continue
    seen.add(key)
    pending.extend(nodes[key]['dependencies'])
index = []
for key in sorted(seen, key=lambda k: (packages[k]['name'], packages[k]['version'])):
    package = packages[key]
    if package['source'] is None:
        continue
    source = Path(package['manifest_path']).parent
    candidates = set()
    for pattern in ('LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*'):
        candidates.update(p for p in source.glob(pattern) if p.is_file())
    if package.get('license_file'):
        candidates.add(source / package['license_file'])
    # unicode-ident requires the additional Unicode notice, not MIT alone.
    if not candidates:
        raise SystemExit('No distributable license file for ' + package['name'])
    folder = out / 'rust' / (package['name'] + '-' + package['version'])
    folder.mkdir(parents=True, exist_ok=True)
    for item in sorted(candidates):
        shutil.copy2(item, folder / item.name)
    index.append({'name': package['name'], 'version': package['version'],
                  'license': package['license'], 'repository': package.get('repository')})
(out / 'rust-dependencies.json').write_text(json.dumps(index, indent=2) + '\n')
sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
rust_docs = sysroot / 'share/doc/rust'
rust_notices = out / 'rust-runtime'
rust_notices.mkdir(parents=True, exist_ok=True)
for item in ('COPYRIGHT.html', 'COPYRIGHT-library.html'):
    shutil.copy2(rust_docs / item, rust_notices / item)
shutil.copytree(rust_docs / 'licenses', rust_notices / 'licenses', dirs_exist_ok=True)
print(f'Collected {len(index)} Rust dependency notice sets and two SDK license notices')
