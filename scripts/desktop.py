#!/usr/bin/env python3
"""Build, test and package the Flutter application with its matching Rust binaries."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import zipfile
import urllib.request
import re
import json
import tomllib
from contextlib import contextmanager

ROOT = Path(__file__).resolve().parents[1]
SYSTEM = {'Linux': 'linux', 'Windows': 'windows', 'Darwin': 'macos'}[platform.system()]

def run(*args, env=None):
    subprocess.run(args, cwd=ROOT, env=env, check=True)

def version():
    return next(line.split(':', 1)[1].strip().split('+')[0] for line in (ROOT / 'pubspec.yaml').read_text().splitlines() if line.startswith('version:'))

def native(release=False):
    rust_version = tomllib.loads((ROOT / 'rust/Cargo.toml').read_text())['package']['version']
    if rust_version != version(): raise ValueError('Flutter and Rust package versions must match')
    run('cargo', 'build', '--locked', '--manifest-path', 'rust/Cargo.toml', *(['--release'] if release else []))
    return ROOT / 'rust' / 'target' / ('release' if release else 'debug')

def library(directory):
    return directory / {'linux': 'libgamehandler_desktop.so', 'windows': 'gamehandler_desktop.dll', 'macos': 'libgamehandler_desktop.dylib'}[SYSTEM]

def bundle(release):
    mode = 'release' if release else 'debug'
    if SYSTEM == 'linux':
        arch = 'arm64' if platform.machine() in ('aarch64', 'arm64') else 'x64'
        return ROOT / 'build' / 'linux' / arch / mode / 'bundle'
    if SYSTEM == 'windows':
        return ROOT / 'build' / 'windows' / 'x64' / 'runner' / mode.capitalize()
    return ROOT / 'build' / 'macos' / 'Build' / 'Products' / mode.capitalize() / 'GameHandler.app'

@contextmanager
def build_lock():
    directory = ROOT / 'build'; directory.mkdir(exist_ok=True)
    with (directory / '.desktop-build.lock').open('a+b') as lock:
        if SYSTEM == 'windows':
            import msvcrt
            lock.seek(0); lock.write(b'0'); lock.flush(); lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_LOCK, 1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            yield
        finally:
            if SYSTEM == 'windows':
                lock.seek(0); msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(lock, fcntl.LOCK_UN)

def verified_download(url, checksum, filename):
    cache = ROOT / 'build' / 'third-party'; cache.mkdir(parents=True, exist_ok=True)
    target = cache / filename
    if not target.exists():
        temporary = target.with_suffix('.partial')
        try:
            with urllib.request.urlopen(url, timeout=60) as response, temporary.open('wb') as stream:
                if not response.geturl().startswith('https://'): raise ValueError('Insecure download redirect')
                count = 0
                while block := response.read(1024 * 1024):
                    count += len(block)
                    if count > 100 * 1024 * 1024: raise ValueError('Oversized build dependency')
                    stream.write(block)
            with temporary.open('rb') as stream:
                if hashlib.file_digest(stream, 'sha256').hexdigest() != checksum: raise ValueError('Build dependency checksum mismatch')
            temporary.replace(target)
        finally:
            temporary.unlink(missing_ok=True)
    with target.open('rb') as stream:
        if hashlib.file_digest(stream, 'sha256').hexdigest() != checksum: raise ValueError(f'Corrupt cached dependency: {target}')
    return target

def bundle_notices(assets):
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version=1', '--manifest-path', 'rust/Cargo.toml'], cwd=ROOT, text=True))
    notices = ['GameHandler Rust dependency notices. Flutter dependency licenses are available in the application license viewer.']
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        notices.append(f"\n{package['name']} {package['version']} — {package.get('license') or 'See license file'}\n{package.get('repository') or ''}")
        directory = Path(package['manifest_path']).parent
        candidates = {*directory.glob('LICENSE*'), *directory.glob('COPYING*')}
        if package['name'] == 'bzip2-sys': candidates.add(directory / 'bzip2-1.0.8/LICENSE')
        if package.get('license_file'): candidates.add(directory / package['license_file'])
        for path in sorted(candidates):
            if path.is_file() and path.stat().st_size < 1024 * 1024:
                notices.append(path.read_text(encoding='utf-8', errors='replace'))
    (assets / 'Rust-DEPENDENCY-NOTICES.txt').write_text('\n'.join(notices), encoding='utf-8')

def bundle_dxvk(assets):
    archive = verified_download('https://github.com/doitsujin/dxvk/releases/download/v3.0.2/dxvk-3.0.2.tar.gz', '9c538924110a7cdef871ca36dee218c0774124374ffdeb38af4b76be55bdf7c2', 'dxvk-3.0.2.tar.gz')
    with tarfile.open(archive, 'r:gz') as files:
        for member in files:
            if member.issym() or member.islnk(): raise ValueError('Unexpected linked DXVK archive member')
            parts = member.name.split('/')
            if member.isfile() and len(parts) == 3 and parts[1] in ('x32', 'x64') and re.fullmatch(r'[A-Za-z0-9_.-]+\.dll', parts[2]):
                destination = assets / 'dxvk' / parts[1] / parts[2]
                destination.parent.mkdir(parents=True, exist_ok=True)
                with files.extractfile(member) as source, destination.open('wb') as target: shutil.copyfileobj(source, target)
    if not (assets / 'dxvk/x64/d3d11.dll').is_file(): raise ValueError('DXVK archive is missing its native DLLs')
    license_file = verified_download('https://raw.githubusercontent.com/doitsujin/dxvk/v3.0.2/LICENSE', 'a5cb1a6ded7d2d7e92d550ba28edd21be2d1d4044662b399887351023e30ce64', 'DXVK-LICENSE.txt')
    shutil.copy2(license_file, assets / 'DXVK-LICENSE.txt')

def build(release):
    with build_lock():
        return build_unlocked(release)

def build_unlocked(release):
    output = native(release)
    run('flutter', 'build', SYSTEM, '--release' if release else '--debug')
    destination = bundle(release)
    libs = destination / 'lib' if SYSTEM == 'linux' else destination / 'Contents' / 'Frameworks' if SYSTEM == 'macos' else destination
    bins = destination / 'Contents' / 'MacOS' if SYSTEM == 'macos' else destination
    libs.mkdir(parents=True, exist_ok=True)
    shutil.copy2(library(output), libs)
    shutil.copy2(output / ('gamehandler-cli.exe' if SYSTEM == 'windows' else 'gamehandler-cli'), bins)
    assets = destination / 'share' / 'gamehandler' if SYSTEM != 'macos' else destination / 'Contents' / 'Resources' / 'gamehandler'
    assets.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / 'data' / 'microsoft-identity-verification-root-ca-2020.pem', assets)
    shutil.copy2(ROOT / 'LICENSE', assets / 'GameHandler-LICENSE.txt')
    shutil.copy2(ROOT / 'data/icons/hicolor/256x256/apps/com.goshapps.GameHandler.png', assets / 'app_icon.png')
    bundle_notices(assets)
    if SYSTEM == 'linux' and platform.machine() == 'x86_64': bundle_dxvk(assets)
    if SYSTEM == 'macos':
        run('/usr/libexec/PlistBuddy', '-c', f'Set :CFBundleShortVersionString {version().split("-")[0]}', str(destination / 'Contents/Info.plist'))
        run('codesign', '--force', '--deep', '--sign', '-', '--entitlements', str(ROOT / 'macos/Runner' / ('Release.entitlements' if release else 'DebugProfile.entitlements')), str(destination))
    return destination

def package(destination):
    release_version = version()
    dist = ROOT / 'dist'; dist.mkdir(exist_ok=True)
    name = f'gamehandler-{release_version}-{SYSTEM}-{platform.machine()}'
    if SYSTEM == 'linux':
        artifact = dist / f'{name}.tar.gz'
        with tarfile.open(artifact, 'w:gz') as archive: archive.add(destination, arcname=name)
    elif SYSTEM == 'macos':
        artifact = dist / f'{name}.zip'
        run('ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(destination), str(artifact))
    else:
        artifact = dist / f'{name}.zip'
        with zipfile.ZipFile(artifact, 'w', zipfile.ZIP_DEFLATED) as archive:
            for path in destination.rglob('*'):
                if path.is_file(): archive.write(path, path.relative_to(destination.parent))
    with artifact.open('rb') as stream:
        checksum = hashlib.file_digest(stream, 'sha256').hexdigest()
    (dist / f'{artifact.name}.sha256').write_text(f'{checksum}  {artifact.name}\n')
    print(artifact)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['native', 'build', 'test', 'run', 'package'])
    parser.add_argument('--release', action='store_true')
    args = parser.parse_args()
    if args.command == 'native': native(args.release)
    elif args.command in ('build', 'package'):
        destination = build(args.release)
        if args.command == 'package': package(destination)
    elif args.command == 'test':
        output = native()
        run('cargo', 'test', '--locked', '--manifest-path', 'rust/Cargo.toml')
        run('cargo', 'clippy', '--locked', '--manifest-path', 'rust/Cargo.toml', '--all-targets', '--all-features', '--', '-D', 'warnings')
        run('flutter', 'analyze')
        run('dart', 'format', '--output=none', '--set-exit-if-changed', 'lib', 'test', 'integration_test')
        run('flutter', 'test', env={**os.environ, 'GAMEHANDLER_NATIVE_LIBRARY': str(library(output))})
    elif args.command == 'run':
        output = native()
        run('flutter', 'run', '-d', SYSTEM, env={**os.environ, 'GAMEHANDLER_NATIVE_LIBRARY': str(library(output))})

if __name__ == '__main__': main()
