#!/usr/bin/env python3
"""Builds and checks opengrid-manifest.json (schema 2) for a release folder.

Every archive in the folder is opened and read: the build class and minimum system its own binaries
declare (OGBuildClass / OGMinOS in Info.plist, or opengrid-build.json), the minimum the Mach-O load
commands really say (per processor type the highest of its binaries, then the lowest of the types), and a SHA-256. The checks are the ones in the plugin QA skill:

  * both fields are present on every archive;
  * build_class agrees with min_os (macOS: modern at 11.0 or later, legacy below; Windows: modern at
    10.0 or later, legacy below);
  * the binary's own minimum (otool -l LC_BUILD_VERSION / LC_VERSION_MIN_MACOSX) equals min_os.

Usage:
  opengrid_manifest.py build  <release-dir> <product> <version>   writes <release-dir>/opengrid-manifest.json
  opengrid_manifest.py verify <release-dir>                       checks it; exit 1 on any FAIL
"""
import hashlib, json, os, plistlib, struct, subprocess, sys, tempfile, shutil, datetime
from pathlib import Path

ARCHIVE_SUFFIXES = ('.pkg', '.dmg', '.zip')


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def version_tuple(text):
    parts = [int(p) for p in str(text).split('.') if p.isdigit()]
    return tuple((parts + [0, 0])[:2])


def version_text(packed):
    return '%d.%d' % ((packed >> 16) & 0xFFFF, (packed >> 8) & 0xFF)


def macho_minimums(path):
    """{arch: 'major.minor'} for the slices of a Mach-O file, or {} when it is not one."""
    try:
        with open(path, 'rb') as f:
            head = f.read(8)
            if len(head) < 8:
                return {}
            magic = struct.unpack('>I', head[:4])[0]
            slices = []
            if magic in (0xCAFEBABE, 0xCAFEBABF):
                count = struct.unpack('>I', head[4:8])[0]
                if count > 8:
                    return {}
                for _ in range(count):
                    entry = f.read(20 if magic == 0xCAFEBABE else 32)
                    cpu, _sub, offset = struct.unpack('>iiI', entry[:12])
                    slices.append((cpu, offset))
            else:
                slices.append((None, 0))
            result = {}
            for cpu, offset in slices:
                f.seek(offset)
                header = f.read(32)
                le = struct.unpack('<I', header[:4])[0]
                if le not in (0xFEEDFACE, 0xFEEDFACF):
                    continue
                cputype = struct.unpack('<i', header[4:8])[0]
                ncmds = struct.unpack('<I', header[16:20])[0]
                f.seek(offset + (28 if le == 0xFEEDFACE else 32))
                commands = f.read(struct.unpack('<I', header[20:24])[0])
                at, minimum = 0, None
                for _ in range(ncmds):
                    cmd, size = struct.unpack_from('<II', commands, at)
                    if cmd == 0x24:
                        minimum = version_text(struct.unpack_from('<I', commands, at + 8)[0])
                    elif cmd == 0x32:
                        minimum = version_text(struct.unpack_from('<I', commands, at + 12)[0])
                    at += size
                name = {7: 'i386', 0x01000007: 'x86_64', 0x0100000C: 'arm64', 18: 'ppc'}.get(cputype, str(cputype))
                if minimum:
                    result[name] = minimum
            return result
    except (OSError, struct.error):
        return {}


def pe_subsystem(path):
    """(OS major.minor) from a PE header, or None."""
    try:
        with open(path, 'rb') as f:
            data = f.read(4096)
        if data[:2] != b'MZ':
            return None
        pe = struct.unpack_from('<I', data, 0x3C)[0]
        if data[pe:pe + 4] != b'PE\0\0':
            return None
        opt = pe + 24
        return '%d.%d' % struct.unpack_from('<HH', data, opt + 40)
    except (OSError, struct.error):
        return None


def stamps_in(root):
    """Declared (class, min_os) pairs from Info.plist files and opengrid-build.json under root."""
    found = []
    for path in root.rglob('*'):
        try:
            if path.name == 'Info.plist' and path.is_file():
                info = plistlib.loads(path.read_bytes())
                if 'OGBuildClass' in info or 'OGMinOS' in info:
                    found.append((info.get('OGBuildClass'), info.get('OGMinOS'), str(path.relative_to(root))))
            elif path.name == 'opengrid-build.json' and path.is_file():
                info = json.loads(path.read_text())
                found.append((info.get('build_class'), info.get('min_os'), str(path.relative_to(root))))
        except Exception:
            continue
    return found


def macho_files(root):
    for path in root.rglob('*'):
        if path.is_file() and not path.is_symlink() and path.stat().st_size > 4096:
            mins = macho_minimums(path)
            if mins:
                yield path, mins


def extract(archive, work):
    """Unpacks an archive; a dmg is mounted and its pkg or zip is opened in turn. Returns the folder."""
    out = Path(work) / (archive.name + '.d')
    out.mkdir(parents=True, exist_ok=True)
    if archive.suffix == '.pkg':
        shutil.rmtree(out)
        subprocess.run(['pkgutil', '--expand-full', str(archive), str(out)], check=True, stdout=subprocess.DEVNULL)
    elif archive.suffix == '.zip':
        subprocess.run(['ditto', '-x', '-k', str(archive), str(out)], check=True)
    elif archive.suffix == '.dmg':
        mount = Path(work) / (archive.name + '.mnt')
        mount.mkdir()
        subprocess.run(['hdiutil', 'attach', '-nobrowse', '-readonly', '-mountpoint', str(mount), str(archive)], check=True, stdout=subprocess.DEVNULL)
        try:
            for inner in sorted(mount.glob('*.pkg')):
                sub = extract(inner, work)
                shutil.copytree(sub, out / inner.name, dirs_exist_ok=True)
        finally:
            subprocess.run(['hdiutil', 'detach', '-force', str(mount)], stdout=subprocess.DEVNULL)
    return out


def inspect(archive, work):
    root = extract(archive, work)
    stamps = stamps_in(root)
    classes = sorted({s[0] for s in stamps if s[0]})
    minimums = sorted({s[1] for s in stamps if s[1]}, key=version_tuple)
    # What the whole archive needs: on each kind of processor the program set runs only where its most
    # demanding binary runs, and the archive runs wherever at least one kind of processor is served.
    per_arch, arches = {}, set()
    for path, mins in macho_files(root):
        for arch, minimum in mins.items():
            arches.add(arch)
            if arch not in per_arch or version_tuple(minimum) > version_tuple(per_arch[arch]):
                per_arch[arch] = minimum
    detected = min(per_arch.values(), key=version_tuple) if per_arch else None
    windows_min = None
    for path in root.rglob('*'):
        if path.suffix.lower() in ('.exe', '.dll') and path.is_file():
            value = pe_subsystem(path)
            if value and (windows_min is None or version_tuple(value) < version_tuple(windows_min)):
                windows_min = value
    return {
        'classes': classes,
        'min_os_declared': minimums[0] if minimums else None,
        'stamps': len(stamps),
        'min_os_binaries': detected,
        'windows_pe_min': windows_min,
        'arch': sorted(arches),
        'platform': 'macos' if detected else ('windows' if windows_min else 'unknown'),
    }


def rule_class(platform, min_os):
    bound = (11, 0) if platform == 'macos' else (10, 0)
    return 'modern' if version_tuple(min_os) >= bound else 'legacy'


def check(entry):
    """List of problems for one archive entry (empty = PASS)."""
    problems = []
    if not entry.get('build_class'):
        problems.append('build_class is missing')
    if not entry.get('min_os'):
        problems.append('min_os is missing')
    if problems:
        return problems
    expected = rule_class(entry['platform'], entry['min_os'])
    if entry['build_class'] != expected:
        problems.append('build_class %s does not agree with min_os %s (%s %s would be %s)' % (entry['build_class'], entry['min_os'], entry['platform'], entry['min_os'], expected))
    if entry['platform'] == 'macos':
        detected = entry.get('min_os_binaries')
        if detected is None:
            problems.append('no Mach-O binaries found to check min_os against')
        elif version_tuple(detected) != version_tuple(entry['min_os']):
            problems.append('the binaries say %s but min_os is %s' % (detected, entry['min_os']))
    elif entry['platform'] == 'windows':
        pe = entry.get('windows_pe_min')
        if pe and version_tuple(pe) > version_tuple(entry['min_os']) and version_tuple(entry['min_os']) < (6, 1):
            problems.append('the executable needs Windows %s, above min_os %s' % (pe, entry['min_os']))
    return problems


def build(release, product, version):
    release = Path(release)
    entries = []
    with tempfile.TemporaryDirectory() as work:
        for archive in sorted(p for p in release.iterdir() if p.suffix in ARCHIVE_SUFFIXES):
            info = inspect(archive, work)
            declared = info['classes']
            entry = {
                'file': archive.name,
                'sha256': sha256(archive),
                'size': archive.stat().st_size,
                'platform': info['platform'],
                'arch': info['arch'],
                'build_class': declared[0] if len(declared) == 1 else None,
                'min_os': info['min_os_declared'],
                'min_os_binaries': info['min_os_binaries'],
                'windows_pe_min': info['windows_pe_min'],
            }
            if len(declared) > 1:
                entry['problem_hint'] = 'the archive mixes classes: ' + ', '.join(declared)
            entries.append(entry)
    classes_by_platform = {(e['platform'], e['build_class']) for e in entries}
    not_published = []
    for platform in ('macos', 'windows', 'linux'):
        for cls in ('modern', 'legacy'):
            if (platform, cls) not in classes_by_platform:
                not_published.append({'platform': platform, 'build_class': cls})
    manifest = {
        'schema': 2,
        'product': product,
        'version': version,
        'generated': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'archives': entries,
        'not_published': not_published,
    }
    (release / 'opengrid-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest


def verify(release):
    release = Path(release)
    manifest = json.loads((release / 'opengrid-manifest.json').read_text())
    failures = 0
    if manifest.get('schema') != 2:
        print('FAIL manifest: schema is not 2')
        failures += 1
    listed = {e['file'] for e in manifest['archives']}
    for archive in sorted(p for p in release.iterdir() if p.suffix in ARCHIVE_SUFFIXES):
        if archive.name not in listed:
            print('FAIL %s: not in the manifest' % archive.name)
            failures += 1
    for entry in manifest['archives']:
        path = release / entry['file']
        problems = check(entry)
        if entry.get('problem_hint'):
            problems.append(entry['problem_hint'])
        if not path.exists():
            problems.append('file is missing')
        elif sha256(path) != entry['sha256']:
            problems.append('SHA-256 does not match')
        label = '%s (%s %s, min %s)' % (entry['build_class'], entry['platform'], ','.join(entry.get('arch') or []), entry['min_os'])
        if problems:
            failures += 1
            print('FAIL %s: %s -> %s' % (entry['file'], label, '; '.join(problems)))
        else:
            print('PASS %s: %s' % (entry['file'], label))
    for item in manifest.get('not_published', []):
        print('NOT PUBLISHED: %s %s' % (item['build_class'], item['platform']))
    return failures


if __name__ == '__main__':
    if len(sys.argv) >= 5 and sys.argv[1] == 'build':
        build(sys.argv[2], sys.argv[3], sys.argv[4])
        sys.exit(1 if verify(sys.argv[2]) else 0)
    if len(sys.argv) == 3 and sys.argv[1] == 'verify':
        sys.exit(1 if verify(sys.argv[2]) else 0)
    print(__doc__)
    sys.exit(2)
