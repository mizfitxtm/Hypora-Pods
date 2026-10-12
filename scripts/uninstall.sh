#!/bin/sh
# Removes what install.sh added. Pairing keys and the state file are deleted too.
set -eu

PREFIX="${PREFIX:-$HOME/.local}"

# On Hypora, hypora-plugin owns the widget files and removes them itself.
[ -n "${SKIP_WIDGET:-}" ] || "$PREFIX/bin/hypora-pods-ctl" uninstall-widget ${WIDGET_DIR:+"$WIDGET_DIR"} 2>/dev/null || true
systemctl --user disable --now hypora-pods.service 2>/dev/null || true
rm -f "$HOME/.config/systemd/user/hypora-pods.service" "$PREFIX/bin/hypora-pods" "$PREFIX/bin/hypora-pods-ctl"
systemctl --user daemon-reload
rm -rf "$HOME/.config/hypora-pods" "$HOME/.local/state/hypora-pods"
echo "removed"
