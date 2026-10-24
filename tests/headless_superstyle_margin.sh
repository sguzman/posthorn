#!/usr/bin/env bash
# Mako v1.11.0 superstyle outer-margin screenshot acceptance.
# Runs ONLY with a disposable Wayland compositor and caller's private D-Bus.
set -euo pipefail
root="$(mktemp -d)"
sway_pid=""
posthorn_pid=""
cleanup(){
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
printf 'xwayland disable\noutput * resolution 1280x720 scale 1\n' >"$root/sway.conf"
sway --unsupported-gpu -c "$root/sway.conf" >"$root/sway.log" 2>&1 &
sway_pid=$!
socket=""
for _ in {1..70}; do
  for candidate in "$XDG_RUNTIME_DIR"/wayland-*; do
    if [[ -S "$candidate" ]]; then socket="$candidate"; break; fi
  done
  [[ -n "$socket" ]] && break
  if ! kill -0 "$sway_pid" 2>/dev/null; then cat "$root/sway.log"; exit 1; fi
  sleep 0.15
done
[[ -n "$socket" ]] || { echo 'No isolated Wayland socket'; cat "$root/sway.log"; exit 1; }
export WAYLAND_DISPLAY="${socket##*/}"
cat >"$root/mako.conf" <<'EOF_CONFIG'
width=180
height=50
margin=0
padding=4
border-size=0
format=%s
default-timeout=30000
background-color=#E5923C
# Neither criteria section below will match the notification.
# Mako config.c:apply_superset_style nevertheless takes the maximum
# outer-margin in EACH direction and wayland.c uses it for all surfaces.
[category=never-right]
outer-margin=0,85,0,0
[category=never-top]
outer-margin=32,0,0,0
EOF_CONFIG
./target/debug/posthorn --experimental-private-desktop "$root/mako.conf" \
  >"$root/posthorn.log" 2>&1 &
posthorn_pid=$!
ready=0
for _ in {1..70}; do
  if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
      org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
    ready=1; break
  fi
  if ! kill -0 "$posthorn_pid" 2>/dev/null; then cat "$root/posthorn.log"; exit 1; fi
  sleep 0.15
done
[[ "$ready" == 1 ]] || { echo 'Private D-Bus service was not registered'; cat "$root/posthorn.log"; exit 1; }
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn superstyle QA' 0 '' 'Unmatched criteria margins' '' 0 0 30000 >/dev/null
painted=0
for _ in {1..55}; do
  if grim -o HEADLESS-1 "$root/popup.png" 2>/dev/null; then
    if bounds="$(./target/debug/examples/qa_png_probe \
        "$root/popup.png" '#E5923C' 200 bounds 2>/dev/null)"; then
      read -r x0 x1 y0 y1 <<<"$bounds"
      # 1280px right-anchored output: 85px superset right margin.
      # The other unmatched criteria contributes the 32px top margin.
      if (( x1 >= 1193 && x1 <= 1195 && y0 >= 31 && y0 <= 33 )); then
        painted=1; break
      fi
    fi
  fi
  if ! kill -0 "$posthorn_pid" 2>/dev/null; then
    echo 'Private daemon exited during superstyle check'
    cat "$root/posthorn.log"
    exit 1
  fi
  sleep 0.15
done
[[ "$painted" == 1 ]] || {
  echo "Mako superstyle placement mismatch: expected right=85/top=32; observed bounds=${bounds:-missing}"
  cat "$root/posthorn.log"
  exit 1
}
echo "Mako v1.11.0 unmatched-criteria superset outer margin: right 85, top 32 OK"
