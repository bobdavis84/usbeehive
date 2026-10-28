#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_SRC="${SCRIPT_DIR}/src-tauri/target/release/usbeehive-desktop"
ICONS_SRC="${SCRIPT_DIR}/src-tauri/icons"
DESKTOP_SRC="${SCRIPT_DIR}/usbeehive.desktop"

# Target user directories
USER_BIN="${HOME}/.local/bin"
USER_APPS="${HOME}/.local/share/applications"
USER_ICONS="${HOME}/.local/share/icons/hicolor"
USER_PIXMAPS="${HOME}/.local/share/pixmaps"
DESKTOP_DIR="${HOME}/Desktop"

echo "==> Creating user directories..."
mkdir -p "${USER_BIN}"
mkdir -p "${USER_APPS}"
mkdir -p "${USER_PIXMAPS}"
mkdir -p "${USER_ICONS}/scalable/apps"
mkdir -p "${USER_ICONS}/512x512/apps"
mkdir -p "${USER_ICONS}/256x256/apps"
mkdir -p "${USER_ICONS}/128x128/apps"
mkdir -p "${USER_ICONS}/64x64/apps"
mkdir -p "${USER_ICONS}/48x48/apps"
mkdir -p "${USER_ICONS}/32x32/apps"

echo "==> Installing binary to ${USER_BIN}/usbeehive-desktop..."
cp "${BIN_SRC}" "${USER_BIN}/usbeehive-desktop"
chmod +x "${USER_BIN}/usbeehive-desktop"

echo "==> Installing icons..."
cp "${ICONS_SRC}/usbeehive.svg" "${USER_ICONS}/scalable/apps/usbeehive.svg"
cp "${ICONS_SRC}/usbeehive-512.png" "${USER_ICONS}/512x512/apps/usbeehive.png"
cp "${ICONS_SRC}/usbeehive-256.png" "${USER_ICONS}/256x256/apps/usbeehive.png"
cp "${ICONS_SRC}/usbeehive-128.png" "${USER_ICONS}/128x128/apps/usbeehive.png"
cp "${ICONS_SRC}/usbeehive-64.png" "${USER_ICONS}/64x64/apps/usbeehive.png"
cp "${ICONS_SRC}/usbeehive-48.png" "${USER_ICONS}/48x48/apps/usbeehive.png"
cp "${ICONS_SRC}/usbeehive-32.png" "${USER_ICONS}/32x32/apps/usbeehive.png"
cp "${ICONS_SRC}/usbeehive-256.png" "${USER_PIXMAPS}/usbeehive.png"

echo "==> Installing desktop entry..."
cp "${DESKTOP_SRC}" "${USER_APPS}/usbeehive.desktop"
chmod +x "${USER_APPS}/usbeehive.desktop"

# Also place a copy on ~/Desktop if directory exists
if [ -d "${DESKTOP_DIR}" ]; then
  cp "${DESKTOP_SRC}" "${DESKTOP_DIR}/usbeehive.desktop"
  chmod +x "${DESKTOP_DIR}/usbeehive.desktop"
  # In GNOME, allow-launching marks it trusted
  if command -v gio >/dev/null 2>&1; then
    gio set "${DESKTOP_DIR}/usbeehive.desktop" "metadata::trusted" yes || true
  fi
fi

# Refresh desktop databases if tools are present
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${USER_APPS}" || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "${USER_ICONS}" || true
fi

echo "==> Installation complete!"
