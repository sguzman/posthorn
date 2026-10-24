#!/usr/bin/env bash
# Isolated two-virtual-output acceptance for Posthorn's opt-in
# private multi-surface renderer. Never uses the user's Wayland/bus session.
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
  if ! kill -0 "$sway_pid" 2>/dev/null; then
    cat "$root/sway.log"
    exit 1
  fi
  sleep 0.15
done
[[ -n "$socket" ]] || { echo 'Two-output Sway socket unavailable'; cat "$root/sway.log"; exit 1; }
export WAYLAND_DISPLAY="${socket##*/}"
ipc=""
for _ in {1..60}; do
  ipc="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
  if [[ -n "$ipc" && -S "$ipc" ]] &&
     swaymsg -s "$ipc" -t get_outputs >/dev/null 2>&1; then break; fi
  sleep 0.15
done
[[ -n "$ipc" && -S "$ipc" ]] || { echo 'Two-output Sway IPC unavailable'; cat "$root/sway.log"; exit 1; }
# Pin the two virtual outputs to explicit non-overlapping logical rectangles
# so the virtual-pointer test has deterministic compositor coordinates.
swaymsg -s "$ipc" 'output HEADLESS-1 pos 0 0' >/dev/null
swaymsg -s "$ipc" 'output HEADLESS-2 pos 1280 0' >/dev/null
cat >"$root/multi-output.conf" <<'EOF_CONFIG'
width=180
height=62
margin=4
border-size=0
default-timeout=30000
output=HEADLESS-1
background-color=#E5923C
[category=second]
output=HEADLESS-2
background-color=#2DB3C1
EOF_CONFIG
./target/debug/posthorn --experimental-private-multi-desktop "$root/multi-output.conf" \
  >"$root/posthorn.log" 2>&1 &
posthorn_pid=$!
ready=0
for _ in {1..60}; do
  if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
     org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "$posthorn_pid" 2>/dev/null; then
    echo 'Private multi-output daemon quit before D-Bus ready'
    cat "$root/posthorn.log"
    exit 1
  fi
  sleep 0.15
done
[[ "$ready" == 1 ]] || { echo 'Private multi-output D-Bus unavailable'; cat "$root/posthorn.log"; exit 1; }
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn multi-output QA' 0 '' 'On monitor one' '' 0 0 30000 >/dev/null
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn multi-output QA' 0 '' 'On monitor two' '' 0 1 \
  'category' 's' 'second' 30000 >/dev/null
correct=0
for _ in {1..60}; do
  if grim -o HEADLESS-1 "$root/one.png" 2>/dev/null &&
     grim -o HEADLESS-2 "$root/two.png" 2>/dev/null &&
     ./target/debug/examples/qa_png_probe "$root/one.png" '#E5923C' 300 present >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe "$root/two.png" '#2DB3C1' 300 present >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe "$root/one.png" '#2DB3C1' 100 absent >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe "$root/two.png" '#E5923C' 100 absent >/dev/null 2>&1
  then
    correct=1
    break
  fi
  if ! kill -0 "$posthorn_pid" 2>/dev/null; then
    echo 'Private multi-output renderer exited'
    cat "$root/posthorn.log"
    exit 1
  fi
  sleep 0.15
done
if [[ "$correct" != 1 ]]; then
  echo 'Notifications were not isolated to their configured Wayland outputs'
  cat "$root/posthorn.log"
  exit 1
fi
# A compositor pointer event in unused space on HEADLESS-2 must not
# activate either independently mapped notification.
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer 1500 350 2560 720 273
grim -o HEADLESS-1 "$root/one-after-gap-click.png"
grim -o HEADLESS-2 "$root/two-after-gap-click.png"
./target/debug/examples/qa_png_probe \
  "$root/one-after-gap-click.png" '#E5923C' 300 present >/dev/null
./target/debug/examples/qa_png_probe \
  "$root/two-after-gap-click.png" '#2DB3C1' 300 present >/dev/null
# Right-click the second monitor's actual popup with the compositor's
# virtual-pointer protocol. The notification on monitor one must stay mapped.
# This exercises surface-local hit-testing and actual pointer event routing,
# not only unit-test coordinate geometry.
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer 2440 20 2560 720 273
independent_click=0
for _ in {1..50}; do
  if grim -o HEADLESS-1 "$root/one-after-click.png" 2>/dev/null &&
     grim -o HEADLESS-2 "$root/two-after-click.png" 2>/dev/null &&
     ./target/debug/examples/qa_png_probe \
       "$root/one-after-click.png" '#E5923C' 300 present >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe \
       "$root/two-after-click.png" '#2DB3C1' 100 absent >/dev/null 2>&1
  then
    independent_click=1
    break
  fi
  if ! kill -0 "$posthorn_pid" 2>/dev/null; then
    echo 'Multi-output renderer exited during pointer delivery'
    cat "$root/posthorn.log"
    exit 1
  fi
  sleep 0.15
done
if [[ "$independent_click" != 1 ]]; then
  echo 'Second-output pointer did not dismiss only its own notification'
  cat "$root/posthorn.log"
  exit 1
fi
# Repeat in the other direction. A click on HEADLESS-1 must not
# dismiss a newly mapped HEADLESS-2 popup. Restore HEADLESS-1 afterward
# so that the output-unplug check still has a surviving mapped surface.
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn multi-output QA' 0 '' 'Second popup restored' '' 0 1 \
  'category' 's' 'second' 30000 >/dev/null
second_restored=0
for _ in {1..50}; do
  if grim -o HEADLESS-2 "$root/two-restored.png" 2>/dev/null &&
     ./target/debug/examples/qa_png_probe \
       "$root/two-restored.png" '#2DB3C1' 300 present >/dev/null 2>&1
  then
    second_restored=1
    break
  fi
  sleep 0.15
done
[[ "$second_restored" == 1 ]] || { echo 'Second output did not remap after pointer dismissal'; cat "$root/posthorn.log"; exit 1; }
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer 1200 20 2560 720 273
reverse_click=0
for _ in {1..50}; do
  if grim -o HEADLESS-1 "$root/one-after-reverse.png" 2>/dev/null &&
     grim -o HEADLESS-2 "$root/two-after-reverse.png" 2>/dev/null &&
     ./target/debug/examples/qa_png_probe \
       "$root/one-after-reverse.png" '#E5923C' 100 absent >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe \
       "$root/two-after-reverse.png" '#2DB3C1' 300 present >/dev/null 2>&1
  then
    reverse_click=1
    break
  fi
  if ! kill -0 "$posthorn_pid" 2>/dev/null; then
    echo 'Multi-output renderer exited during reverse pointer delivery'
    cat "$root/posthorn.log"
    exit 1
  fi
  sleep 0.15
done
[[ "$reverse_click" == 1 ]] || { echo 'First-output pointer did not dismiss only its own popup'; cat "$root/posthorn.log"; exit 1; }
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn multi-output QA' 0 '' 'First popup restored' '' 0 0 30000 >/dev/null
first_restored=0
for _ in {1..50}; do
  if grim -o HEADLESS-1 "$root/one-restored.png" 2>/dev/null &&
     ./target/debug/examples/qa_png_probe \
       "$root/one-restored.png" '#E5923C' 300 present >/dev/null 2>&1
  then
    first_restored=1
    break
  fi
  sleep 0.15
done
[[ "$first_restored" == 1 ]] || { echo 'First output did not remap after pointer dismissal'; cat "$root/posthorn.log"; exit 1; }
# Output loss is not a daemon shutdown. The surviving monitor must retain
# its notification and the private control D-Bus name.
swaymsg -s "$ipc" 'output HEADLESS-2 unplug' >/dev/null
sleep 0.3
kill -0 "$posthorn_pid"
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s introspect \
  org.posthorn.Development /org/freedesktop/Notifications >/dev/null
grim -o HEADLESS-1 "$root/after-unplug.png"
./target/debug/examples/qa_png_probe \
  "$root/after-unplug.png" '#E5923C' 300 present >/dev/null
echo 'Independent two-output routing, gap and bidirectional pointer isolation, popup remapping, and output unplug: OK'
