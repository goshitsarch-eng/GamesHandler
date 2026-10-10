#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/desktop.py build --release
flatpak --user remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean --repo=dist/flatpak-repo build/flatpak packaging/flatpak/com.goshapps.GameHandler.json
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo dist/flatpak-repo dist/gamehandler-0.9.0-alpha.1.flatpak com.goshapps.GameHandler
flatpak --user install --noninteractive dist/gamehandler-0.9.0-alpha.1.flatpak
flatpak run --command=gamehandler-cli com.goshapps.GameHandler --version
flatpak run --command=gamehandler-cli com.goshapps.GameHandler --list
