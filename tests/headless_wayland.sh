#!/usr/bin/env bash
# Automated development-only Wayland acceptance smoke. Never owns the
# desktop org.freedesktop.Notifications name or touches a user's session.
set -euo pipefail

root="$(mktemp -d)"
sway_pid=""
posthorn_pid=""
cleanup() {
  if [[ -n "$posthorn_pid" ]]; then kill "$posthorn_pid" 2>/dev/null || true; wait "$posthorn_pid" 2>/dev/null || true; fi
  if [[ -n "$sway_pid" ]]; then kill "$sway_pid" 2>/dev/null || true; wait "$sway_pid" 2>/dev/null || true; fi
  rm -rf "$root"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$root/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export WLR_BACKENDS=headless
export WLR_HEADLESS_OUTPUTS=1
export WLR_RENDERER=pixman
export XDG_CURRENT_DESKTOP=sway

printf 'output * resolution 1280x720\n' > "$root/sway.conf"
sway --unsupported-gpu -c "$root/sway.conf" >"$root/sway.log" 2>&1 &
sway_pid=$!

socket=""
for _ in {1..60}; do
  for candidate in "$XDG_RUNTIME_DIR"/wayland-*; do
    if [[ -S "$candidate" ]]; then socket="$candidate"; break; fi
  done
  [[ -n "$socket" ]] && break
  if ! kill -0 "$sway_pid" 2>/dev/null; then
    echo 'Headless Sway terminated before creating Wayland socket'
    cat "$root/sway.log"
    exit 1
  fi
  sleep 0.2
done
if [[ -z "$socket" ]]; then
  echo 'No Wayland socket from headless Sway'
  cat "$root/sway.log"
  exit 1
fi
export WAYLAND_DISPLAY="${socket##*/}"

cat > "$root/mako.conf" <<'EOF'
width=300
height=120
margin=8
default-timeout=7000
[category=success]
background-color=#305747
EOF

./target/debug/posthorn --experimental-private-desktop "$root/mako.conf" >"$root/posthorn.log" 2>&1 &
posthorn_pid=$!

for _ in {1..60}; do
  if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$posthorn_pid" 2>/dev/null; then
    echo 'Posthorn terminated before private D-Bus registration'
    cat "$root/posthorn.log"
    cat "$root/sway.log"
    exit 1
  fi
  sleep 0.2
done
if ! busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
  echo 'Private D-Bus notification service did not become available'
  cat "$root/posthorn.log"
  cat "$root/sway.log"
  exit 1
fi

result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
  /org/freedesktop/Notifications org.freedesktop.Notifications Notify \
  'susssasa{sv}i' 'Posthorn Wayland smoke' 0 '' 'Hello' 'A real Wayland popup' 0 0 7000)"
[[ "$result" == u\ * ]] || { echo "Invalid Notify result: $result"; exit 1; }
sleep 0.5
if ! kill -0 "$posthorn_pid" 2>/dev/null; then
  echo 'Posthorn terminated while rendering the notification'
  cat "$root/posthorn.log"
  cat "$root/sway.log"
  exit 1
fi
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s introspect org.posthorn.Development /fr/emersion/Mako >/dev/null
echo 'Headless Sway + private Posthorn D-Bus + live layer-shell popup: OK'
