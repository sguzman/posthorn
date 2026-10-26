#!/usr/bin/env bash
# Synthetic bus-identity regression: never runs against the real desktop.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == 'true' ]] || {
  echo 'Synthetic parent-bus probe is CI-only' >&2
  exit 1
}
root="$(mktemp -d)"
trap 'rm -rf -- "$root"' EXIT
mkdir -p "$root/bin" "$root/runtime"
cat >"$root/bin/busctl" <<'EOF_BUSCTL'
#!/usr/bin/env bash
[[ " $* " == *" GetId "* ]] || exit 89
case "${1:-}" in
  --user) printf 's "original-user-bus-guid"\\n' ;;
  --address=*) printf 's "mismatched-pinned-bus-guid"\\n' ;;
  *) exit 90 ;;
esac
EOF_BUSCTL
cat >"$root/bin/hyprctl" <<'EOF_HYPRCTL'
#!/usr/bin/env bash
[[ "${1:-}" == monitors ]] || exit 91
EOF_HYPRCTL
for tool in cargo grim dbus-run-session; do
  cat >"$root/bin/$tool" <<'EOF_DENY'
#!/usr/bin/env bash
echo 'A compositor or build operation was reached unexpectedly' >&2
exit 92
EOF_DENY
done
chmod +x "$root/bin/"*
if env PATH="$root/bin:$PATH" POSTHORN_HYPRLAND_QA=1 \
    HYPRLAND_INSTANCE_SIGNATURE=synthetic WAYLAND_DISPLAY=wayland-synthetic \
    XDG_RUNTIME_DIR="$root/runtime" DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent \
    bash tests/hyprland_private.sh >"$root/output" 2>&1; then
  echo 'Mismatched pinned bus unexpectedly passed private QA preflight' >&2
  exit 1
fi
if ! grep -q 'pinned D-Bus address is not the original user bus' "$root/output"; then
  cat "$root/output" >&2
  echo 'Private QA did not reject the mismatched bus GUID' >&2
  exit 1
fi
echo 'PASS: mismatched pinned bus refused before build or compositor mutation'
