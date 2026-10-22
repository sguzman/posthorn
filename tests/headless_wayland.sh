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

# Exercise the same D-Bus / compositor acceptance at 1x and 2x.
# The larger 2x output keeps four notifications visible at once.
test_scale="${POSTHORN_TEST_SCALE:-1}"
case "$test_scale" in 1) resolution="1280x720";; 2) resolution="1920x1080";;
  *) echo "Unsupported acceptance test scale: $test_scale"; exit 1;; esac
printf 'output * resolution %s scale %s\n' "$resolution" "$test_scale" > "$root/sway.conf"
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
[category=failure]
background-color=#684148
[category=warning]
background-color=#715536
EOF

WAYLAND_DEBUG=client ./target/debug/posthorn --experimental-private-desktop "$root/mako.conf" >"$root/posthorn.log" 2>&1 &
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
# Observe real Wayland protocol traffic, not merely D-Bus success
# or an alive process. The compositor must configure the layer surface
# and Posthorn must attach a real shared-memory notification buffer.
painted=0
for _ in {1..30}; do
  if grep -Eq 'zwlr_layer_surface_v1@.*configure' "$root/posthorn.log" &&
     grep -Eq 'wl_surface@.*attach' "$root/posthorn.log"; then
    painted=1
    break
  fi
  sleep 0.2
done
if [[ "$painted" != 1 ]]; then
  echo 'No configured layer-shell popup with attached buffer'
  cat "$root/posthorn.log"
  cat "$root/sway.log"
  exit 1
fi
if ! kill -0 "$posthorn_pid" 2>/dev/null; then
  echo 'Posthorn terminated while rendering the notification'
  cat "$root/posthorn.log"
  cat "$root/sway.log"
  exit 1
fi
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s introspect org.posthorn.Development /fr/emersion/Mako >/dev/null

# A second message proves the notification category reaches the renderer.
# 1 is the a{sv} length; the variant is (key='category', type=s, value=success).
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
  /org/freedesktop/Notifications org.freedesktop.Notifications Notify \
  'susssasa{sv}i' 'Posthorn screenshot' 0 '' 'Success' '' 0 1 'category' 's' 'success' 7000 >/dev/null
warning_id=""
for category in failure warning; do
  result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
    /org/freedesktop/Notifications org.freedesktop.Notifications Notify \
    'susssasa{sv}i' 'Posthorn screenshot' 0 '' "$category" '' 0 1 'category' 's' "$category" 7000)"
  if [[ "$category" == warning ]]; then warning_id="${result#u }"; fi
done
[[ "$warning_id" =~ ^[0-9]+$ ]] || { echo "Invalid warning notification ID: $warning_id"; exit 1; }
sleep 0.5
grim "$root/popup.png"
/usr/bin/python3 - "$root/popup.png" <<'PY'
from collections import Counter
from PIL import Image
import sys

pixels = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
expected = {
    "success": (48, 87, 71),
    "failure": (104, 65, 72),
    "warning": (113, 85, 54),
}
top_rows = {}
image = Image.open(sys.argv[1]).convert("RGB")
width, _ = image.size
for category, rgb in expected.items():
    assert pixels[rgb] >= 300, (
        f"No {category} category popup color visible on headless Sway; "
        f"found {pixels[rgb]} pixels of {rgb}"
    )
    hits = [divmod(index, width)
            for index, value in enumerate(image.getdata()) if value == rgb]
    top_rows[category] = min(row for row, _ in hits)
    assert min(col for _, col in hits) > width // 2, (
        f"{category} popup was not anchored near the right of the output"
    )
    print(f"Visual color smoke: {category} = {pixels[rgb]} pixels "
          f"starting at y={top_rows[category]}")
assert top_rows["warning"] < top_rows["failure"] < top_rows["success"], (
    f"Mako newest-first stack order changed: {top_rows}"
)
PY
# Close one popup through the actual Freedesktop method. The compositor must
# repaint the stack, removing that color without erasing the remaining popups.
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
  /org/freedesktop/Notifications org.freedesktop.Notifications CloseNotification \
  'u' "$warning_id" >/dev/null
cat > "$root/check-dismissed.py" <<'PY'
from collections import Counter
from PIL import Image
import sys

counts = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
warning = counts[(113, 85, 54)]
success = counts[(48, 87, 71)]
failure = counts[(104, 65, 72)]
if warning >= 100 or success < 300 or failure < 300:
    raise SystemExit(
        f"Dismiss/repaint mismatch: warning={warning}, "
        f"success={success}, failure={failure}"
    )
PY
dismiss_repainted=0
for _ in {1..30}; do
  sleep 0.2
  grim "$root/after-close.png"
  if /usr/bin/python3 "$root/check-dismissed.py" "$root/after-close.png" >/dev/null 2>&1; then
    dismiss_repainted=1
    break
  fi
done
if [[ "$dismiss_repainted" != 1 ]]; then
  /usr/bin/python3 "$root/check-dismissed.py" "$root/after-close.png"
  echo 'Notification close did not repaint the live Wayland stack'
  cat "$root/posthorn.log"
  exit 1
fi
# Exercise timer-driven expiry through the full private D-Bus / Wayland path.
# Use a distinct warning color so visual removal can be distinguished from
# ordinary notification closure in the compositor capture.
expiry_result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
  /org/freedesktop/Notifications org.freedesktop.Notifications Notify \
  'susssasa{sv}i' 'Posthorn expiry smoke' 0 '' 'Temporary warning' '' 0 1 'category' 's' 'warning' 1400)"
[[ "$expiry_result" == u\ * ]] || { echo "Invalid expiring Notify result: $expiry_result"; exit 1; }
appeared=0
for _ in {1..8}; do
  sleep 0.1
  grim "$root/before-expiry.png"
  if /usr/bin/python3 - "$root/before-expiry.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(113, 85, 54)] >= 300 else 1)
PY
  then appeared=1; break; fi
done
if [[ "$appeared" != 1 ]]; then
  echo 'Timed notification never appeared before expiry'
  exit 1
fi
sleep 1.4
expired=0
for _ in {1..20}; do
  sleep 0.2
  grim "$root/after-expiry.png"
  if /usr/bin/python3 "$root/check-dismissed.py" "$root/after-expiry.png" >/dev/null 2>&1; then
    expired=1
    break
  fi
done
if [[ "$expired" != 1 ]]; then
  echo 'Timed notification did not disappear from compositor screenshot'
  /usr/bin/python3 "$root/check-dismissed.py" "$root/after-expiry.png"
  exit 1
fi
# Reload must repaint active Wayland popups without replacing the D-Bus
# service, and a rejected reload must preserve the last good style.
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
  /org/freedesktop/Notifications org.freedesktop.Notifications Notify \
  'susssasa{sv}i' 'Posthorn reload smoke' 0 '' 'Repaint me' '' 0 1 \
  'category' 's' 'warning' 20000 >/dev/null
old_visible=0
for _ in {1..15}; do
  sleep 0.15
  grim "$root/before-reload.png"
  if /usr/bin/python3 - "$root/before-reload.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
pixels = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if pixels[(113, 85, 54)] >= 300 else 1)
PY
  then old_visible=1; break; fi
done
[[ "$old_visible" == 1 ]] || { echo 'Reload test did not paint original color'; exit 1; }

sed -i 's/background-color=#715536/background-color=#234C8F/' "$root/mako.conf"
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
  /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
new_visible=0
for _ in {1..20}; do
  sleep 0.15
  grim "$root/after-reload.png"
  if /usr/bin/python3 - "$root/after-reload.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
pixels = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if pixels[(35,76,143)] >= 300
    and pixels[(113,85,54)] < 100 else 1)
PY
  then new_visible=1; break; fi
done
[[ "$new_visible" == 1 ]] || {
  echo 'Reload did not repaint the active notification in the new category color'
  cat "$root/posthorn.log"
  exit 1
}

sed -i 's/background-color=#234C8F/background-color=not-a-color/' "$root/mako.conf"
if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload \
  >"$root/invalid-reload.log" 2>&1; then
  echo 'Invalid reload was unexpectedly accepted'
  exit 1
fi
sleep 0.25
grim "$root/after-invalid-reload.png"
/usr/bin/python3 - "$root/after-invalid-reload.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
pixels = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
assert pixels[(35,76,143)] >= 300, (
    "Invalid reload erased the last successfully committed notification style"
)
PY
sed -i 's/background-color=not-a-color/background-color=#234C8F/' "$root/mako.conf"
# A mode switch, unlike a configuration reload, must immediately
# restyle already-active popups without restarting the daemon. Exercise the
# actual layer-shell pixels via the private development bus at both scales.
printf '\n[mode=quiet]\nbackground-color=#6B488F\n' >> "$root/mako.conf"
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
  's' quiet >/dev/null
mode_painted=0
for _ in {1..20}; do
  sleep 0.15
  grim "$root/quiet-mode.png"
  if /usr/bin/python3 - "$root/quiet-mode.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(107,72,143)] >= 300 else 1)
PY
  then mode_painted=1; break; fi
done
[[ "$mode_painted" == 1 ]] || {
  echo 'SetMode quiet did not repaint the active popup'
  cat "$root/posthorn.log"
  exit 1
}
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
  's' default >/dev/null
mode_reverted=0
for _ in {1..20}; do
  sleep 0.15
  grim "$root/default-mode.png"
  if /usr/bin/python3 - "$root/default-mode.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(35,76,143)] >= 300
    and counts[(107,72,143)] < 100 else 1)
PY
  then mode_reverted=1; break; fi
done
[[ "$mode_reverted" == 1 ]] || {
  echo 'SetMode default did not restore the existing popup appearance'
  cat "$root/posthorn.log"
  exit 1
}
echo "Headless Sway ${test_scale}x + D-Bus popups, timer, reload and live modes: OK"

if [[ "$test_scale" == 1 ]]; then
  # Live registry hotplug: the target named output does not exist at daemon
  # startup. Posthorn must keep its private D-Bus name and queued notification,
  # then paint on the new output when the compositor announces it.
  ipc_socket="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
  [[ -S "$ipc_socket" ]] || { echo "Could not find isolated Sway IPC socket"; exit 1; }
  kill "$posthorn_pid"
  wait "$posthorn_pid" 2>/dev/null || true
  posthorn_pid=""
  sed -i '1i output=HEADLESS-2' "$root/mako.conf"
  ./target/debug/posthorn --experimental-private-desktop "$root/mako.conf" \
    >"$root/hotplug-posthorn.log" 2>&1 &
  posthorn_pid=$!
  ready=0
  for _ in {1..40}; do
    if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
      org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 0.1
  done
  [[ "$ready" == 1 ]] || { echo "Private service did not restart for hotplug"; exit 1; }

  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
    /org/freedesktop/Notifications org.freedesktop.Notifications Notify \
    'susssasa{sv}i' 'Posthorn hotplug' 0 '' 'Hotplug success' '' 0 1 \
    'category' 's' 'success' 7000 >/dev/null
  sleep 0.25
  kill -0 "$posthorn_pid" || { echo "Posthorn crashed while output missing"; exit 1; }

  swaymsg -s "$ipc_socket" create_output >/dev/null
  hotplug_painted=0
  for _ in {1..40}; do
    sleep 0.2
    if grim -o HEADLESS-2 "$root/hotplug-popup.png" 2>/dev/null && \
      /usr/bin/python3 - "$root/hotplug-popup.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(48, 87, 71)] >= 300 else 1)
PY
    then
      hotplug_painted=1
      break
    fi
    if ! kill -0 "$posthorn_pid" 2>/dev/null; then
      echo 'Posthorn exited after headless output was added'
      cat "$root/hotplug-posthorn.log"
      exit 1
    fi
  done
  if [[ "$hotplug_painted" != 1 ]]; then
    echo 'Posthorn did not render on the newly hotplugged named output'
    cat "$root/hotplug-posthorn.log"
    exit 1
  fi
  swaymsg -s "$ipc_socket" 'output HEADLESS-2 unplug' >/dev/null
  sleep 0.25
  kill -0 "$posthorn_pid" || { echo "Posthorn exited on output unplug"; exit 1; }
  echo 'Named output hotplug: missing -> added -> popup drawn -> removed, daemon alive: OK'
fi
