#!/usr/bin/env python3
"""Check the actual bundle's ABI and headless launcher before distributing it."""
import argparse
import ctypes
import subprocess
import tempfile
import os
from desktop import bundle, SYSTEM, version

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--release', action='store_true')
args = parser.parse_args()
root = bundle(args.release)
lib = root / 'lib/libgamehandler_desktop.so' if SYSTEM == 'linux' else root / 'Contents/Frameworks/libgamehandler_desktop.dylib' if SYSTEM == 'macos' else root / 'gamehandler_desktop.dll'
backend = ctypes.CDLL(str(lib))
assert backend.gamehandler_abi_version() == 1
cli = root / 'Contents/MacOS/gamehandler-cli' if SYSTEM == 'macos' else root / ('gamehandler-cli.exe' if SYSTEM == 'windows' else 'gamehandler-cli')
with tempfile.TemporaryDirectory(prefix='gamehandler-bundle-') as directory:
    env = {**os.environ, 'GAMEHANDLER_CONFIG_HOME': directory, 'GAMEHANDLER_DATA_HOME': directory}
    result = subprocess.run([str(cli), '--version'], env=env, check=True, capture_output=True, text=True)
    assert version() in result.stdout
    subprocess.run([str(cli), '--list'], env=env, check=True)
    result = subprocess.run([str(cli), '--launch', 'missing'], env=env, capture_output=True)
    assert result.returncode != 0
print(f'Validated ABI and CLI: {root}')
