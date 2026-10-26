#!/usr/bin/env bash
# Isolated upstream/Posthorn geometry differential. No user desktop/bus.
set -euo pipefail
[[ "$POSTHORN_PINNED_DIFFERENTIAL" == 1 ]] || { echo 'explicit guard required'; exit 1; }
[[ -v DBUS_SESSION_BUS_ADDRESS ]] || { echo 'private D-Bus required'; exit 1; }
[[ ! -v WAYLAND_DISPLAY && ! -v HYPRLAND_INSTANCE_SIGNATURE ]] || {
  echo 'refuse existing Wayland desktop'; exit 1;
}
mako_binary="$1"
[[ -x "$mako_binary" ]] || { echo 'pinned Mako binary missing'; exit 1; }
root="$(mktemp -d)"
sway_pid=""
daemon_pid=""
cleanup() {
  if [[ -n "$daemon_pid" ]]; then kill "$daemon_pid" 2>/dev/null || true; wait "$daemon_pid" 2>/dev/null || true; fi
  if [[ -n "$sway_pid" ]]; then kill "$sway_pid" 2>/dev/null || true; wait "$sway_pid" 2>/dev/null || true; fi
  rm -rf "$root"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$root/runtime" WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1
export WLR_RENDERER=pixman XDG_CURRENT_DESKTOP=sway
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
printf 'xwayland disable\noutput * resolution 1280x720 scale 1\n' >"$root/sway.conf"
sway --unsupported-gpu -c "$root/sway.conf" >"$root/sway.log" 2>&1 &
sway_pid=$!
socket=""
for _ in {1..60}; do
  for f in "$XDG_RUNTIME_DIR"/wayland-*; do
    if [[ -S "$f" ]]; then socket="$f"; break; fi
  done
  [[ -n "$socket" ]] && break
  if ! kill -0 "$sway_pid" 2>/dev/null; then cat "$root/sway.log"; exit 1; fi
  sleep 0.2
done
[[ -n "$socket" ]] || { cat "$root/sway.log"; exit 1; }
export WAYLAND_DISPLAY="$(basename "$socket")"
ready=0
for _ in {1..60}; do
  ipc="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
  if [[ -S "$ipc" ]] && swaymsg -s "$ipc" -t get_outputs >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 0.2
done
[[ "$ready" == 1 ]] || { cat "$root/sway.log"; exit 1; }
cat >"$root/mako.conf" <<'EOF_CONFIG'
width=84
height=270
font=Sans 20
margin=0
padding=0
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%s
background-color=#315E31
[hidden]
width=260
height=270
format=MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM
background-color=#9361B2
EOF_CONFIG
grim "$root/baseline.png"
./target/debug/examples/qa_png_probe "$root/baseline.png" '#9361B2' 1 absent >/dev/null
wait_for_bus() {
  local name="$1"
  for _ in {1..60}; do
    if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
      "$name" /org/freedesktop/Notifications >/dev/null 2>&1; then return 0; fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then cat "$root/daemon.log"; return 1; fi
    sleep 0.15
  done
  echo "service $name did not register"; cat "$root/daemon.log"; return 1
}
notify_pair() {
  local name="$1" title response
  for title in 'First hidden' 'Second visible'; do
    response="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      "$name" /org/freedesktop/Notifications \
      org.freedesktop.Notifications Notify 'susssasa{sv}i' \
      'Pinned Mako geometry differential' 0 '' "$title" '' 0 0 30000)"
    [[ "$response" == u\ * ]] || { echo "Notify failed: $response"; return 1; }
  done
}
capture_bounds() {
  local label="$1" bounds=""
  for _ in {1..45}; do
    if grim "$root/$label.png" 2>/dev/null; then
      bounds="$(./target/debug/examples/qa_png_probe \
        "$root/$label.png" '#9361B2' 250 bounds 2>/dev/null || true)"
      if [[ -n "$bounds" ]]; then echo "$bounds"; return 0; fi
    fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then cat "$root/daemon.log" >&2; return 1; fi
    sleep 0.15
  done
  echo "missing hidden label for $label" >&2
  cat "$root/daemon.log" >&2
  return 1
}
./target/debug/posthorn --experimental-private-desktop "$root/mako.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
notify_pair org.posthorn.Development
post_bounds="$(capture_bounds posthorn)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
cleared=0
for _ in {1..50}; do
  sleep 0.1
  if grim "$root/cleared.png" 2>/dev/null &&
    ./target/debug/examples/qa_png_probe "$root/cleared.png" '#9361B2' 1 absent \
      >/dev/null 2>&1; then cleared=1; break; fi
done
[[ "$cleared" == 1 ]] || { echo 'Posthorn surface was not removed'; exit 1; }
"$mako_binary" -c "$root/mako.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
notify_pair org.freedesktop.Notifications
mako_bounds="$(capture_bounds mako)"
read -r px0 px1 py0 py1 <<< "$post_bounds"
read -r mx0 mx1 my0 my1 <<< "$mako_bounds"
pw=$((px1-px0+1)); ph=$((py1-py0+1))
mw=$((mx1-mx0+1)); mh=$((my1-my0+1))
echo "Posthorn hidden bounds: $post_bounds ($pw x $ph)"
echo "Pinned Mako hidden bounds: $mako_bounds ($mw x $mh)"
(( pw<=84 && mw<=84 && pw>=70 && mw>=70 && ph>=100 && mh>=100 )) || {
  echo 'invalid or unexpectedly small comparison regions'; exit 1;
}
(( (pw-mw)<=2 && (mw-pw)<=2 && (ph-mh)<=2 && (mh-ph)<=2 )) || {
  echo 'Source-pinned Mako/Posthorn hidden-label geometry differs'; exit 1;
}
echo 'Source-pinned Mako/Posthorn hidden-label geometry: MATCH'

# Non-gating observation of upstream Mako's ambiguous max-visible=0
# synthetic surface. render.c reports width=0 but nonzero height;
# wayland.c may attempt a zero-width layer-shell configure. Do NOT
# silently turn this into a passing source-parity claim. Observe the
# actual pinned executable and record whether the pixel/color remains.
sed -i 's/^max-visible=1$/max-visible=0/' "$root/mako.conf"
if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.freedesktop.Notifications /fr/emersion/Mako fr.emersion.Mako Reload \
    >"$root/hidden-zero-reload.log" 2>&1; then
  sleep 0.5
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    echo 'UPSTREAM OBSERVATION max-visible=0: Mako exited after successful Reload'
    cat "$root/daemon.log"
  elif grim "$root/hidden-zero.png" 2>/dev/null; then
    bounds="$(./target/debug/examples/qa_png_probe \
      "$root/hidden-zero.png" '#9361B2' 1 bounds 2>/dev/null || true)"
    if [[ -n "$bounds" ]]; then
      echo "UPSTREAM OBSERVATION max-visible=0: hidden-only pixels at $bounds"
    else
      echo 'UPSTREAM OBSERVATION max-visible=0: no hidden-only label pixels'
    fi
  else
    echo 'UPSTREAM OBSERVATION max-visible=0: screenshot unavailable'
  fi
else
  echo 'UPSTREAM OBSERVATION max-visible=0: Reload failed; inspect daemon log'
  cat "$root/hidden-zero-reload.log"
  cat "$root/daemon.log"
fi
