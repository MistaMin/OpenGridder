#!/usr/bin/env python3
"""Stage an existing server binary and a double-clickable macOS launcher.

No rebuilding, signing or SDK copying. CI must state its actual hosting capability.
"""
import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--capabilities', choices=['au-only', 'au-vst3'], required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    if binary.parent == output:
        raise SystemExit('Stage into a separate directory to preserve the original build')
    version = subprocess.check_output([str(binary), '--version'], text=True).strip()
    if ('VST3=true' in version) != (args.capabilities == 'au-vst3'):
        raise SystemExit('Requested capability label does not match the compiled server')
    if 'AU=true' not in version:
        raise SystemExit('This launcher stages the macOS AU host only')
    output.mkdir(parents=True, exist_ok=True)
    destination = output / 'OpenGridderServer'
    shutil.copy2(binary, destination)
    destination.chmod(0o755)
    launcher = output / 'Start OpenGridder Server.command'
    launcher.write_text('''#!/bin/bash
set -euo pipefail
og_directory="$(cd "$(dirname "$0")" && pwd)"
printf '%s\\n' 'Starting OpenGridder Server. Keep this Terminal window open.'
printf '%s\\n' 'The first scan of a large plugin collection can take several minutes.'
exec "$og_directory/OpenGridderServer" "$@"
''')
    launcher.chmod(0o755)
    capability = 'AU and VST3' if args.capabilities == 'au-vst3' else 'AU only (VST3 SDK not included in hosted CI)'
    (output / 'START-HERE.txt').write_text(f'''OpenGridder Server — development build
{version}
Hosting: {capability}

Double-click "Start OpenGridder Server.command" to start the server locally.
Keep Terminal open; press Control-C to stop it. A first full plugin scan can take
several minutes. For a quick functional check, run this command in Terminal:

./OpenGridderServer --au AudioUnit:aufx:lpas:appl

The default address is 127.0.0.1:55056. For a separate studio machine use:
./OpenGridderServer --bind 0.0.0.0 --port 55056
Remote mode is intended for a trusted studio network; there is no protocol authentication.

The AAX client reads ~/Library/Application Support/OpenGridder/client.json:
{{"server":"127.0.0.1:55056","transport_samples":256}}

Real AU/VST3 effects are supported subject to the capability above. This build
is not a finished production system: native editor capture, full AAX session
recall/automation, sidechains, instruments and old OS compatibility remain open.
No RTAS/TDM .dpm files are included. Pro Tools requires the separately signed AAX
client; a server binary alone is not a Pro Tools plugin.
''')
    sums = []
    for file in sorted(output.iterdir()):
        if file.is_file() and file.name != 'SHA256SUMS':
            sums.append(f'{hashlib.sha256(file.read_bytes()).hexdigest()}  {file.name}')
    (output / 'SHA256SUMS').write_text('\n'.join(sums) + '\n')
    print(output)

if __name__ == '__main__':
    main()
