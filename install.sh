#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

if ! command -v gpu-screen-recorder >/dev/null; then
    echo "gpu-screen-recorder não encontrado. Instale antes: sudo pacman -S gpu-screen-recorder" >&2
    exit 1
fi

cargo build --release

bin="$HOME/.local/bin/ahri"
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor/scalable/apps"
units="$HOME/.config/systemd/user"

install -Dm755 target/release/ahri "$bin"
install -Dm644 assets/icon.svg "$icons/ahri.svg"
install -Dm644 packaging/ahri.service "$units/ahri.service"
mkdir -p "$apps"
cat > "$apps/ahri.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Ahri
Comment=Print e gravação de tela
Exec=$bin
Icon=ahri
NoDisplay=true
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
DESKTOP

kbuildsycoca6 >/dev/null 2>&1 || true
systemctl --user daemon-reload
systemctl --user enable ahri.service
systemctl --user restart ahri.service

echo "ahri instalado. Win+Shift+S para print, Win+R para gravar."
