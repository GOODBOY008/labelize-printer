#!/usr/bin/env python3
"""Bundle the actual dependency notices alongside trial installers."""
import os, pathlib, shutil, tomllib
root = pathlib.Path(__file__).resolve().parents[1]
registry = pathlib.Path(os.environ.get('CARGO_HOME', pathlib.Path.home()/'.cargo')) / 'registry/src'
out = root / 'licenses'; out.mkdir(exist_ok=True)
rows = []
for package in tomllib.loads((root/'Cargo.lock').read_text(encoding='utf-8'))['package']:
    if package['name'].startswith('labelize-printer'): continue
    if package['name'] in ('labelize','xattr','gpui'): directory = root/'vendor'/package['name']
    else:
        candidates = list(registry.glob(f"*/{package['name']}-{package['version']}"))
        if not candidates: continue  # Target-specific packages not included in this build.
        directory = candidates[0]
    info = tomllib.loads((directory/'Cargo.toml').read_text(encoding='utf-8'))['package']
    name = f"{package['name']}-{package['version']}"
    files = [p for p in directory.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE','LICENCE','COPYING','NOTICE'))]
    for p in files:
        dest = out / name / p.name; dest.parent.mkdir(exist_ok=True); shutil.copyfile(p, dest)
    rows.append(f"| {name} | {info.get('license') or 'See bundled license'} | {'; '.join(p.name for p in files) or 'See upstream repository'} |")
for source, name in [(root/'.cache/pappl-1.4.12/LICENSE','PAPPL-1.4.12.txt'),
                     (root/'.cache/openssl-3.5.9/LICENSE.txt','OpenSSL-3.5.9.txt'),
                     (root/'vendor/gpui/PATCH.md','GPUI-patch.md'),
                     (root/'vendor/xattr/PATCH.md','xattr-patch.md'),
                     (root/'vendor/labelize/THIRD_PARTY_LICENSES.md','Labelize-assets.md')]:
    if source.exists(): shutil.copyfile(source, out/name)
(out/'INDEX.md').write_text('# Third-party notices\n\nGenerated from Cargo.lock and dependency sources present for this build.\n\n'
    'PAPPL 1.4.12: Apache-2.0 with LLVM exception; bundled header-preservation patch is in scripts/build-native.sh. '
    'OpenSSL 3.5.9: Apache-2.0. CUPS and operating-system frameworks are dynamically linked system libraries. '
    'ZDesigner is installed separately from Zebra and is not redistributed.\n\n'
    '| Dependency | License expression | Bundled notice files |\n|---|---|---|\n' + '\n'.join(sorted(rows)) + '\n',
    encoding='utf-8')
