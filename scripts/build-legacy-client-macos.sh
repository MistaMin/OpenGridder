#!/bin/bash
# Build the complete SDK-neutral host client with the last Apple i386 Rust.
# An archive, not a loadable AAX/RTAS/TDM plugin. No SDK source is copied.
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
og_crate="$og_root/native/legacy/client-core"
og_old="${OG_LEGACY_TOOLCHAIN:-$HOME/.rustup/toolchains/1.41.1-x86_64-apple-darwin}"
og_arch="$(uname -m)"
if [ "$og_arch" = arm64 ]; then og_arch=aarch64; fi
og_new="${OG_MODERN_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-$og_arch-apple-darwin}"
og_vendor="$og_root/target/legacy-client-vendor"
og_build="$og_root/target/legacy-client-build"
og_output="$og_root/target/legacy-client-i686"
test -x "$og_old/bin/rustc" && test -x "$og_new/bin/cargo"
mkdir -p "$og_vendor" "$og_build/.cargo" "$og_output"
# The modern Cargo fetches exact locked crates; old Cargo never accesses the
# retired Git index or parses a new lockfile. Only generated build space is used.
RUSTC="$og_new/bin/rustc" "$og_new/bin/cargo" vendor --locked \
    --manifest-path "$og_crate/Cargo.toml" --versioned-dirs "$og_vendor" > "$og_build/vendor-config.txt"
cp "$og_crate/Cargo.toml" "$og_build/Cargo.toml"
mkdir -p "$og_build/src"
cp "$og_crate"/src/*.rs "$og_build/src/"
cat > "$og_build/link-host.sh" <<'LINK'
#!/usr/bin/env python3
# Rust 1.41 rlibs contain metadata members rejected by Apple's new linker.
# Strip metadata only from generated link copies, never the installed toolchain.
import hashlib,os,shutil,subprocess,sys,tempfile
from pathlib import Path
cache=Path(__file__).parent/'host-link-archives'
cache.mkdir(exist_ok=True)
arguments=[]
for value in sys.argv[1:]:
    source=Path(value)
    if value.endswith('.rlib') and source.is_file():
        stat=source.stat()
        key=hashlib.sha256((str(source)+str(stat.st_mtime_ns)+str(stat.st_size)).encode()).hexdigest()[:16]
        target=cache/('native-only-v2-'+key+'-'+source.name)
        if not target.exists():
            descriptor,temporary=tempfile.mkstemp(dir=cache,suffix='.rlib'); os.close(descriptor)
            with tempfile.TemporaryDirectory(dir=cache) as extracted:
                subprocess.run(['/usr/bin/ar','x',str(source)],cwd=extracted,check=True)
                objects=sorted(Path(extracted).glob('*.o'))
                if not objects: raise SystemExit('No native objects in '+str(source))
                filelist=Path(extracted)/'objects.txt'
                filelist.write_text(''.join(str(p)+'\n' for p in objects))
                subprocess.run(['/usr/bin/libtool','-static','-no_warning_for_no_symbols',
                    '-filelist',str(filelist),'-o',temporary],check=True,stdout=subprocess.DEVNULL)
            os.replace(temporary,target)
        value=str(target)
    arguments.append(value)
os.execv('/usr/bin/clang',['clang','-arch','x86_64']+arguments)
LINK
chmod +x "$og_build/link-host.sh"
python3 - "$og_vendor" "$og_build/.cargo/config" "$og_build/link-host.sh" <<'PY'
import json,sys
from pathlib import Path
Path(sys.argv[2]).write_text('[source.crates-io]\nreplace-with = "og-legacy-vendor"\n[source.og-legacy-vendor]\ndirectory = '+json.dumps(sys.argv[1])+'\n[target.x86_64-apple-darwin]\nlinker = '+json.dumps(sys.argv[3])+'\n')
PY
(
    cd "$og_build"
    RUSTC="$og_old/bin/rustc" /usr/bin/arch -x86_64 "$og_old/bin/cargo" build \
        --offline --release --target i686-apple-darwin --target-dir "$og_output/build"
    if [ "${OG_LEGACY_RUN_TESTS:-0}" = 1 ]; then
        RUSTC="$og_old/bin/rustc" /usr/bin/arch -x86_64 "$og_old/bin/cargo" test \
            --offline --lib --target x86_64-apple-darwin --target-dir "$og_output/build"
        if [ -n "${OPENGRIDDER_TEST_SERVER:-}" ]; then
            RUSTC="$og_old/bin/rustc" /usr/bin/arch -x86_64 "$og_old/bin/cargo" test \
                --offline --lib --target x86_64-apple-darwin --target-dir "$og_output/build" -- --ignored
        fi
    fi
)
cp "$og_output/build/i686-apple-darwin/release/libopengridder_legacy_client.a" "$og_output/"
lipo "$og_output/libopengridder_legacy_client.a" -verify_arch i386
nm -g "$og_output/libopengridder_legacy_client.a" > "$og_output/symbols.txt" 2> "$og_output/nm-warnings.txt"
python3 - "$og_output/symbols.txt" <<'PY'
from pathlib import Path
import re,sys
text=Path(sys.argv[1]).read_text()
for name in ('create','destroy','process','select','bypass','latency','accept_latency','status','name','default_latency'):
    if not re.search(r' T _og_native_'+name+r'$',text,re.M):
        raise SystemExit('Missing legacy ABI export: '+name)
PY
cp "$og_root/LICENSE" "$og_output/LICENSE.txt"
cat > "$og_output/README.txt" <<'TEXT'
OpenGridder legacy Rust client - i386 host archive, Rust 1.41.1
Contains real v13 discovery/control/audio transport and preallocated callback ABI.
This is not a loadable AAX or .dpm plugin. Wrapper linking requires an external
Apple i386 SDK, AAX/RTAS SDK inputs and legacy host validation. TDM additionally
needs working DSP firmware and host/card DMA integration.
TEXT
mkdir -p "$og_output/Licenses"
mkdir -p "$og_output/Licenses/rust-runtime"
for og_notice in COPYRIGHT LICENSE-APACHE LICENSE-MIT; do
    cp "$og_old/share/doc/rust/$og_notice" "$og_output/Licenses/rust-runtime/"
done
python3 - "$og_vendor" "$og_output/Licenses" <<'PY'
import shutil,sys
from pathlib import Path
for package in Path(sys.argv[1]).iterdir():
    if not package.is_dir(): continue
    for pattern in ('LICENSE*','COPYING*','NOTICE*'):
        for source in package.glob(pattern):
            if source.is_file():
                target=Path(sys.argv[2])/package.name/source.name
                target.parent.mkdir(parents=True,exist_ok=True)
                shutil.copy2(source,target)
PY
(cd "$og_output" && shasum -a 256 libopengridder_legacy_client.a > SHA256SUMS)
echo "Built complete i386 host client archive: $og_output/libopengridder_legacy_client.a"
