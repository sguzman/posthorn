#!/usr/bin/env bash
# Isolated Sway frame evidence for pinned Mako signed-padding semantics.
# No real session bus, installed Mako daemon, or workstation compositor used.
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
[[ -n "$socket" ]] || { echo 'Private Sway socket not available'; cat "$root/sway.log"; exit 1; }
export WAYLAND_DISPLAY="${socket##*/}"
cat >"$root/mako.conf" <<'EOF_CONFIG'
width=180
height=150
border-size=0
margin=0
padding=0
format=%s
default-timeout=30000
background-color=#E5923C
[mode=compact]
padding=-3,0,-2,0
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
[[ "$ready" == 1 ]] || { echo 'Private Posthorn D-Bus unavailable'; cat "$root/posthorn.log"; exit 1; }
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn signed-padding QA' 0 '' 'Text visible' '' 0 0 30000 >/dev/null
bounds(){
  local output="$1"
  for _ in {1..45}; do
    if grim -o HEADLESS-1 "$root/$output.png" 2>/dev/null; then
      local bbox
      if bbox="$(./target/debug/examples/qa_png_probe \
        "$root/$output.png" '#E5923C' 200 bounds 2>/dev/null)"; then
        printf '%s\n' "$bbox"
        return 0
      fi
    fi
    if ! kill -0 "$posthorn_pid" 2>/dev/null; then
      echo 'Posthorn exited before signed-padding screenshot' >&2
      cat "$root/posthorn.log" >&2
      return 1
    fi
    sleep 0.15
  done
  echo 'No painted popup detected in private Sway screenshot' >&2
  cat "$root/posthorn.log" >&2
  return 1
}
read -r x0 x1 y0 y1 <<<"$(bounds plain)"
plain_height=$((y1-y0+1))
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
  's' compact >/dev/null
compact_height=0
for _ in {1..45}; do
  read -r cx0 cx1 cy0 cy1 <<<"$(bounds compact)"
  compact_height=$((cy1-cy0+1))
  if [[ "$compact_height" -eq $((plain_height-5)) ]]; then break; fi
  sleep 0.1
done
[[ "$compact_height" -eq $((plain_height-5)) ]] || {
  echo "Signed-padding popup height mismatch: plain=$plain_height compact=$compact_height"
  cat "$root/posthorn.log"
  exit 1
}
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
  's' default >/dev/null
restored_height=0
for _ in {1..45}; do
  read -r rx0 rx1 ry0 ry1 <<<"$(bounds restored)"
  restored_height=$((ry1-ry0+1))
  if [[ "$restored_height" -eq "$plain_height" ]]; then break; fi
  sleep 0.1
done
[[ "$restored_height" -eq "$plain_height" ]] || {
  echo "Mode reset failed to restore Mako height: plain=$plain_height restored=$restored_height"
  cat "$root/posthorn.log"
  exit 1
}
echo "Private compositor signed-padding mode reflow: $plain_height -> $compact_height -> $restored_height OK"
