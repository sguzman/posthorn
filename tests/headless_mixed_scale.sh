#!/usr/bin/env bash
# Real, isolated Sway compositor acceptance for differently-scaled monitors.
# The 1x notification must choose the largest attached output's themed icon,
# and its pointer input region must follow the resulting *painted* height.
# This test never connects to the workstation Wayland socket or real D-Bus.
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
export WLR_HEADLESS_OUTPUTS=2
export WLR_RENDERER=pixman
export XDG_CURRENT_DESKTOP=sway
printf 'xwayland disable\noutput * resolution 1280x720 scale 1\n' >"$root/sway.conf"
sway --unsupported-gpu -c "$root/sway.conf" >"$root/sway.log" 2>&1 &
sway_pid=$!
socket=""
for _ in {1..60}; do
  for candidate in "$XDG_RUNTIME_DIR"/wayland-*; do
    if [[ -S "$candidate" ]]; then socket="$candidate"; break; fi
  done
  [[ -n "$socket" ]] && break
  if ! kill -0 "$sway_pid" 2>/dev/null; then cat "$root/sway.log"; exit 1; fi
  sleep 0.15
done
[[ -n "$socket" ]] || { echo 'Private mixed-scale Sway socket missing'; cat "$root/sway.log"; exit 1; }
export WAYLAND_DISPLAY="${socket##*/}"
ipc=""
for _ in {1..60}; do
  ipc="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
  if [[ -n "$ipc" && -S "$ipc" ]] &&
    swaymsg -s "$ipc" -t get_outputs >/dev/null 2>&1; then break; fi
  sleep 0.15
done
[[ -n "$ipc" && -S "$ipc" ]] || { echo 'Private mixed-scale Sway IPC unavailable'; cat "$root/sway.log"; exit 1; }
swaymsg -s "$ipc" 'output HEADLESS-1 pos 0 0' >/dev/null
swaymsg -s "$ipc" 'output HEADLESS-2 pos 1280 0' >/dev/null
swaymsg -s "$ipc" 'output HEADLESS-2 scale 2' >/dev/null
POSTHORN_HEADLESS_ICON_TEST=1 ./target/debug/examples/qa_icon_theme "$root/icons"
cat >"$root/mixed.conf" <<EOF_CONFIG
width=180
height=115
margin=4
padding=4
border-size=0
max-icon-size=16
icon-location=top
icon-path=$root/icons
format=%s
default-timeout=30000
output=HEADLESS-1
background-color=#E5923C
EOF_CONFIG
./target/debug/posthorn --experimental-private-multi-desktop "$root/mixed.conf" \
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
[[ "$ready" == 1 ]] || { echo 'Private mixed-scale D-Bus unavailable'; cat "$root/posthorn.log"; exit 1; }
notify(){
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn private mixed-DPI QA' 0 'posthorn-qa' \
    'Scale-sensitive notification' '' 0 0 30000 >/dev/null
}
# Return the actual amber background rectangle from a 1x physical
# screenshot. Green or red are the selected theme icon's solid pixels.
read_bounds(){
  local expected="$1" excluded="$2" file="$3"
  for _ in {1..70}; do
    if grim -o HEADLESS-1 "$file" 2>/dev/null &&
       ./target/debug/examples/qa_png_probe "$file" "$expected" 2 present >/dev/null 2>&1 &&
       ./target/debug/examples/qa_png_probe "$file" "$excluded" 2 absent >/dev/null 2>&1
    then
      local bounds
      if bounds="$(./target/debug/examples/qa_png_probe "$file" '#E5923C' 200 bounds 2>/dev/null)"; then
        printf '%s\n' "$bounds"
        return 0
      fi
    fi
    if ! kill -0 "$posthorn_pid" 2>/dev/null; then
      echo 'Private mixed-scale daemon exited' >&2
      cat "$root/posthorn.log" >&2
      return 1
    fi
    sleep 0.15
  done
  echo "Popup never matched expected theme icon $expected" >&2
  cat "$root/posthorn.log" >&2
  return 1
}
wait_disappearance(){
  local gone=0
  for _ in {1..50}; do
    if grim -o HEADLESS-1 "$root/after-click.png" 2>/dev/null &&
      ./target/debug/examples/qa_png_probe \
        "$root/after-click.png" '#E5923C' 100 absent >/dev/null 2>&1; then
      gone=1; break
    fi
    if ! kill -0 "$posthorn_pid" 2>/dev/null; then cat "$root/posthorn.log"; exit 1; fi
    sleep 0.15
  done
  [[ "$gone" == 1 ]] || { echo 'Pointer missed visible popup bottom edge'; cat "$root/posthorn.log"; exit 1; }
}
# With the second monitor at scale=2, the 1x popup MUST show the green
# @2 theme icon rather than its red 1x sibling.
notify
bounds="$(read_bounds '#00FF00' '#FF0000' "$root/max-2x.png")" || exit 1
read -r x1_min x1_max y1_min y1_max <<<"$bounds"
tall_height=$((y1_max-y1_min+1))
# A live output scale change must NEVER refit the existing icon.
# Mako holds the decoded @2 asset until the notification is replaced.
swaymsg -s "$ipc" 'output HEADLESS-2 scale 1' >/dev/null
bounds="$(read_bounds '#00FF00' '#FF0000' "$root/max-1x-old-retained.png")" || exit 1
read -r old_x_min old_x_max old_y_min old_y_max <<<"$bounds"
[[ $((old_y_max-old_y_min+1)) -eq "$tall_height" ]] || {
  echo 'The already-created @2 icon was unexpectedly resized at monitor change'
  cat "$root/posthorn.log"
  exit 1
}
click_x=$((old_x_min+12))
click_y=$((old_y_max-1))
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer "$click_x" "$click_y" 2560 720 273
wait_disappearance
# A NEW Notify at current maximum scale 1 must select the red, short PNG.
notify
bounds="$(read_bounds '#FF0000' '#00FF00' "$root/new-max-1x.png")" || exit 1
read -r x2_min x2_max y2_min y2_max <<<"$bounds"
short_height=$((y2_max-y2_min+1))
[[ "$tall_height" -ge $((short_height+5)) ]] || {
  echo "New Notify at scale 1 did not pick the shorter asset: tall=$tall_height, short=$short_height"
  cat "$root/posthorn.log"
  exit 1
}
click_x=$((x2_min+12))
click_y=$((y2_max-1))
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer "$click_x" "$click_y" 2560 720 273
wait_disappearance
# Restore the 2x monitor, create another fresh notification, and confirm
# its tall hit region and @2 asset rather than retroactively changing old.
swaymsg -s "$ipc" 'output HEADLESS-2 scale 2' >/dev/null
sleep 0.35
notify
bounds="$(read_bounds '#00FF00' '#FF0000' "$root/new-max-2x.png")" || exit 1
read -r x3_min x3_max y3_min y3_max <<<"$bounds"
[[ $((y3_max-y3_min+1)) -ge $((short_height+5)) ]] || {
  echo 'New Notify at restored 2x scale did not select tall icon'
  cat "$root/posthorn.log"
  exit 1
}
click_x=$((x3_min+12))
click_y=$((y3_max-1))
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer "$click_x" "$click_y" 1920 720 273
wait_disappearance
echo 'Mixed-scale icon lifetime, new arrivals and real pointer routing: OK'
