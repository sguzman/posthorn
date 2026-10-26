#!/usr/bin/env bash
# Automated development-only Wayland acceptance smoke. Never owns the
# desktop org.freedesktop.Notifications name or touches a user's session.
set -euo pipefail

root="$(mktemp -d)"
sway_pid=""
posthorn_pid=""
monitor_pid=""
dual_pid=""
multi_pid=""
cleanup() {
  if [[ -n "$multi_pid" ]]; then kill "$multi_pid" 2>/dev/null || true; wait "$multi_pid" 2>/dev/null || true; fi
  if [[ -n "$dual_pid" ]]; then kill "$dual_pid" 2>/dev/null || true; wait "$dual_pid" 2>/dev/null || true; fi
  if [[ -n "$monitor_pid" ]]; then kill "$monitor_pid" 2>/dev/null || true; wait "$monitor_pid" 2>/dev/null || true; fi
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

# Exercise D-Bus and compositor behavior at 1x, 1.25x, 2x and 9x outputs.
# The larger outputs keep four notifications visible at once. Fractional
# output smoke does not claim wp_fractional_scale_v1 buffer negotiation.
test_scale="${POSTHORN_TEST_SCALE:-1}"
case "$test_scale" in
  1) resolution="1280x720";;
  1.25|2|9) resolution="1920x1080";;
  *) echo "Unsupported acceptance test scale: $test_scale"; exit 1;;
esac
# Compile the Rust virtual pointer QA client before sending any
# notifications so its build time cannot consume a test notification TTL.
if [[ "$test_scale" == 1 ]]; then
  cargo build --quiet --example qa_virtual_pointer
fi
# Keep the headless test Wayland-only; no Xwayland binary is provisioned.
printf 'xwayland disable\noutput * resolution %s scale %s\n' "$resolution" "$test_scale" > "$root/sway.conf"
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
# An early Wayland socket does not mean the compositor has completed startup.
# Wait for the isolated Sway IPC command path before starting Posthorn's
# registry roundtrip; otherwise a busy runner can falsely fail layer-shell QA.
ipc_ready=0
for _ in {1..60}; do
  startup_ipc="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
  if [[ -n "$startup_ipc" && -S "$startup_ipc" ]] &&
      swaymsg -s "$startup_ipc" -t get_outputs >/dev/null 2>&1; then
    ipc_ready=1
    break
  fi
  if ! kill -0 "$sway_pid" 2>/dev/null; then
    echo 'Sway exited before IPC initialization'
    cat "$root/sway.log"
    exit 1
  fi
  sleep 0.15
done
[[ "$ipc_ready" == 1 ]] || {
  echo 'Headless Sway did not finish IPC initialization'
  cat "$root/sway.log"
  exit 1
}

# Verify a genuinely mapped 9x output and the Posthorn compositor
# path independently of all 1x/1.25x/2x fixtures. At 1920x1080 pixels
# Sway exposes only about 213x120 *logical* pixels, so this deliberately
# uses a tiny popup to stay within the output and SHM safety bounds.
if [[ "$test_scale" == 9 ]]; then
  output_info="$(swaymsg -s "$startup_ipc" -t get_outputs)"
  if ! grep -Eq '"scale":[[:space:]]*9(\.0+)?[,}]' <<< "$output_info"; then
    echo 'Headless compositor did not advertise a real 9x output'
    printf '%s\n' "$output_info"
    exit 1
  fi
  cat > "$root/nine.conf" <<'EOF_NINE'
width=64
height=65
margin=1
padding=2
border-size=0
icons=0
max-visible=1
default-timeout=0
on-button-right=dismiss
background-color=#305747
text-color=#FFFFFFFF
EOF_NINE
  grim "$root/nine-baseline.png"
  ./target/debug/posthorn --experimental-private-desktop "$root/nine.conf" \
    >"$root/nine-posthorn.log" 2>&1 &
  posthorn_pid=$!
  ready=0
  for _ in {1..60}; do
    if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
        org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
      ready=1
      break
    fi
    if ! kill -0 "$posthorn_pid" 2>/dev/null; then
      cat "$root/nine-posthorn.log"
      echo '9x private Posthorn exited before D-Bus registration'
      exit 1
    fi
    sleep 0.15
  done
  [[ "$ready" == 1 ]] || { echo '9x private D-Bus registration timed out'; exit 1; }

  reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn 9x QA' 0 '' 'Nine' '' 0 0 30000)"
  [[ "$reply" == u\ * ]] || { echo "9x Notify failed: $reply"; exit 1; }
  id="${reply#u }"
  painted=0
  for _ in {1..45}; do
    if grim "$root/nine-visible.png" 2>/dev/null &&
      ./target/debug/examples/qa_png_probe \
        "$root/nine-visible.png" '#305747' 350 increase "$root/nine-baseline.png" \
        >/dev/null 2>&1; then
      painted=1
      break
    fi
    if ! kill -0 "$posthorn_pid" 2>/dev/null; then
      cat "$root/nine-posthorn.log"
      echo '9x daemon exited before rendering notification'
      exit 1
    fi
    sleep 0.15
  done
  [[ "$painted" == 1 ]] || {
    cat "$root/nine-posthorn.log"
    echo '9x private notification failed compositor pixel acceptance'
    exit 1
  }

  # Read the actual painted bounds instead of guessing Mako's Pango
  # height. PNG coordinates are physical; the virtual pointer expects
  # logical Wayland coordinates, so divide the midpoint by scale 9.
  read -r x0 x1 y0 y1 <<< "$(./target/debug/examples/qa_png_probe \
    "$root/nine-visible.png" '#305747' 350 bounds)"
  click_x=$(( (x0 + x1) / 18 ))
  click_y=$(( (y0 + y1) / 18 ))
  if (( click_x < 0 || click_x >= 213 || click_y < 0 || click_y >= 120 )); then
    echo "9x computed logical click outside output: $click_x,$click_y"
    exit 1
  fi
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer \
    "$click_x" "$click_y" 213 120 273
  # An explicitly sent D-Bus CloseNotification would hide an input-region
  # defect, so require *compositor-originated right click* to dismiss it.
  cleared=0
  for _ in {1..45}; do
    if grim "$root/nine-cleared.png" 2>/dev/null &&
      ./target/debug/examples/qa_png_probe \
        "$root/nine-cleared.png" '#305747' 45 restored "$root/nine-baseline.png" \
        >/dev/null 2>&1; then
      cleared=1
      break
    fi
    sleep 0.15
  done
  [[ "$cleared" == 1 ]] || {
    cat "$root/nine-posthorn.log"
    echo '9x private notification did not disappear after compositor right click'
    exit 1
  }
  listed="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
  if [[ "$listed" == *'Nine'* ]]; then
    echo '9x clicked notification remains active in the private control API'
    exit 1
  fi
  echo 'Actual Sway output scale 9x: private popup rendered, pointer-dismissed and cleared'
  exit 0
fi

# Test two genuine compositor layer surfaces simultaneously. This is a
# development-only Wayland acceptance probe, not the production renderer.
if [[ "$test_scale" == 1 ]]; then
  ./target/debug/examples/qa_dual_surface "$root/dual-ready" \
    >"$root/dual.log" 2>&1 &
  dual_pid=$!
  for _ in {1..70}; do
    [[ -f "$root/dual-ready" ]] && break
    if ! kill -0 "$dual_pid" 2>/dev/null; then
      echo 'Two-surface Wayland probe exited before both surfaces mapped'
      cat "$root/dual.log"
      exit 1
    fi
    sleep 0.05
  done
  if [[ ! -f "$root/dual-ready" ]]; then
    echo 'Two-surface Wayland probe did not receive both wl_surface.enter events'
    cat "$root/dual.log"
    exit 1
  fi
  grim "$root/dual.png"
  ./target/debug/examples/qa_png_probe \
    "$root/dual.png" '#E5923C' 500 present
  ./target/debug/examples/qa_png_probe \
    "$root/dual.png" '#2DB3C1' 500 present
  wait "$dual_pid"
  dual_pid=""
  echo 'Simultaneous independent top-left and bottom-right layer-shell surfaces: OK'
fi

# Exercise simultaneous independent layer-shell notifications on the
# 1x, fractional 1.25x and 2x isolated compositor configurations.
# Output scale is compositor-derived and each presenter owns its buffer.
if [[ "$test_scale" == 1 || "$test_scale" == 1.25 || "$test_scale" == 2 ]]; then
  cat > "$root/multi.conf" <<'EOF_MULTI'
width=160
height=56
margin=3
border-size=0
default-timeout=30000
max-visible=1
background-color=#E5923C
[category=left]
anchor=bottom-left
layer=overlay
background-color=#2DB3C1
EOF_MULTI
  ./target/debug/posthorn --experimental-private-multi-desktop "$root/multi.conf" \
    >"$root/multi-posthorn.log" 2>&1 &
  multi_pid=$!
  multi_ready=0
  for _ in {1..60}; do
    if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
      org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
      multi_ready=1
      break
    fi
    if ! kill -0 "$multi_pid" 2>/dev/null; then
      echo 'Multi-surface private daemon exited before service registration'
      cat "$root/multi-posthorn.log"
      exit 1
    fi
    sleep 0.15
  done
  [[ "$multi_ready" == 1 ]] || {
    echo 'Multi-surface private D-Bus service did not register'
    cat "$root/multi-posthorn.log"
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn multi QA' 0 '' 'Right notification' '' 0 0 30000 >/dev/null
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn multi QA' 0 '' 'Left notification' '' 0 1 \
    'category' 's' 'left' 30000 >/dev/null
  both_painted=0
  for _ in {1..60}; do
    if grim "$root/multi.png" 2>/dev/null \
      && ./target/debug/examples/qa_png_probe \
        "$root/multi.png" '#E5923C' 300 present >/dev/null 2>&1 \
      && ./target/debug/examples/qa_png_probe \
        "$root/multi.png" '#2DB3C1' 300 present >/dev/null 2>&1; then
      both_painted=1
      break
    fi
    if ! kill -0 "$multi_pid" 2>/dev/null; then
      echo 'Multi-surface daemon exited during compositor rendering'
      cat "$root/multi-posthorn.log"
      exit 1
    fi
    sleep 0.15
  done
  if [[ "$both_painted" != 1 ]]; then
    echo 'Two independently anchored notification surfaces were not both painted'
    cat "$root/multi-posthorn.log"
    exit 1
  fi
  kill "$multi_pid"
  wait "$multi_pid" 2>/dev/null || true
  multi_pid=""
  echo 'Private multi-surface D-Bus routing and simultaneous corners: OK'
fi

cat > "$root/mako.conf" <<'EOF'
width=300
height=120
margin=8
default-timeout=30000
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
  'susssasa{sv}i' 'Posthorn Wayland smoke' 0 '' 'Hello' 'A real Wayland popup' 0 0 30000)"
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
  'susssasa{sv}i' 'Posthorn screenshot' 0 '' 'Success' '' 0 1 'category' 's' 'success' 30000 >/dev/null
warning_id=""
for category in failure warning; do
  result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
    /org/freedesktop/Notifications org.freedesktop.Notifications Notify \
    'susssasa{sv}i' 'Posthorn screenshot' 0 '' "$category" '' 0 1 'category' 's' "$category" 30000)"
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
# Exercise compositor-originated right-click dismissal at 1× and 1.25×.
# The virtual pointer uses logical output coordinates (native mode/scale),
# not physical PNG pixel positions; it never touches the user's input seat.
# The 2× smoke retains independent D-Bus close/repaint coverage.
if [[ "$test_scale" == 1 || "$test_scale" == 1.25 ]]; then
  ipc_socket="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
  [[ -S "$ipc_socket" ]] || { echo "Could not find isolated Sway IPC socket"; exit 1; }
  if [[ "$test_scale" == 1 ]]; then
    click_x=1100; logical_width=1280; logical_height=720
  else
    click_x=1350; logical_width=1536; logical_height=864
  fi
  # Sway starts without any physical pointer. Register a temporary wlroots
  # virtual device, then deliver actual wl_pointer enter/press/release events.
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer \
    "$click_x" 25 "$logical_width" "$logical_height"
else
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call org.posthorn.Development \
    /org/freedesktop/Notifications org.freedesktop.Notifications CloseNotification \
    'u' "$warning_id" >/dev/null
fi
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
  /usr/bin/python3 "$root/check-dismissed.py" "$root/after-close.png" || true
  echo 'Notification close did not repaint the live Wayland stack'
  if [[ "$test_scale" == 1 ]]; then
    echo 'Nested compositor seat state:'
    swaymsg -s "$ipc_socket" -t get_seats || true
    echo 'Relevant compositor-delivered pointer events:'
    grep -E 'wl_pointer@.*(enter|motion|button)|wl_seat@.*capabilities' \
      "$root/posthorn.log" | tail -45 || true
  fi
  cat "$root/posthorn.log"
  exit 1
fi
# Retain the existing 2× Freedesktop close/repaint acceptance above,
# then separately exercise an actual 2× compositor-originated right-click.
# The 1920×1080 physical headless output has 960×540 logical seat geometry.
if [[ "$test_scale" == 2 ]]; then
  result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn HiDPI pointer fixture' 0 '' 'Right click at 2x' '' \
    0 1 'category' 's' 'warning' 30000)"
  [[ "$result" == u\ * ]] || { echo 'Invalid 2x action fixture ID'; exit 1; }
  hidpi_appeared=0
  for _ in {1..20}; do
    sleep 0.1
    grim "$root/before-hidpi-pointer.png"
    if /usr/bin/python3 - "$root/before-hidpi-pointer.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
pixels = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if pixels[(113,85,54)] >= 300 else 1)
PY
    then hidpi_appeared=1; break; fi
  done
  [[ "$hidpi_appeared" == 1 ]] || { echo '2x pointer target never painted'; exit 1; }
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 800 25 960 540
  hidpi_dismissed=0
  for _ in {1..20}; do
    sleep 0.15
    grim "$root/after-hidpi-pointer.png"
    if /usr/bin/python3 "$root/check-dismissed.py" "$root/after-hidpi-pointer.png" >/dev/null 2>&1; then
      hidpi_dismissed=1
      break
    fi
  done
  [[ "$hidpi_dismissed" == 1 ]] || {
    echo '2x compositor pointer did not dismiss only the warning popup'
    /usr/bin/python3 "$root/check-dismissed.py" "$root/after-hidpi-pointer.png" || true
    exit 1
  }
fi
# Verify actual default-action/XDG activation at fractional and HiDPI
# scales, independently of the already-covered 1x action path.
if [[ "$test_scale" == 1.25 || "$test_scale" == 2 ]]; then
  if [[ "$test_scale" == 1.25 ]]; then
    action_x=1350; logical_width=1536; logical_height=864
  else
    action_x=800; logical_width=960; logical_height=540
  fi
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" monitor org.posthorn.Development \
    >"$root/scaled-action-signals.log" 2>&1 &
  monitor_pid=$!
  sleep 0.2
  action_reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn scaled action fixture' 0 '' 'Scaled default action' '' \
    2 'default' 'Open' 1 'category' 's' 'warning' 30000)"
  [[ "$action_reply" == u\ * ]] || { echo 'Invalid scaled action notification ID'; exit 1; }
  scaled_action_ready=0
  for _ in {1..20}; do
    sleep 0.12
    grim "$root/scaled-action-before.png"
    if ./target/debug/examples/qa_png_probe \
        "$root/scaled-action-before.png" '#715536' 300 present >/dev/null 2>&1; then
      scaled_action_ready=1; break
    fi
  done
  [[ "$scaled_action_ready" == 1 ]] || { echo 'Scaled action popup never painted'; exit 1; }
  before_tokens="$(grep -Ec 'xdg_activation_token_v1@[^ ]+[.]done' "$root/posthorn.log" || true)"
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer \
    "$action_x" 25 "$logical_width" "$logical_height" 272
  scaled_action_done=0
  for _ in {1..35}; do
    latest_tokens="$(grep -Ec 'xdg_activation_token_v1@[^ ]+[.]done' "$root/posthorn.log" || true)"
    if (( latest_tokens == before_tokens + 1 )) &&
       grep -q 'ActivationToken' "$root/scaled-action-signals.log" &&
       grep -q 'ActionInvoked' "$root/scaled-action-signals.log" &&
       grep -q 'NotificationClosed' "$root/scaled-action-signals.log"; then
      scaled_action_done=1; break
    fi
    sleep 0.12
  done
  token_value="$(sed -nE 's/.*xdg_activation_token_v1@[^ ]+[.]done, [(]Some[(]"([^"]+)"[)].*/\1/p' "$root/posthorn.log" | tail -1)"
  activation_line="$(grep -n 'ActivationToken' "$root/scaled-action-signals.log" | head -1 | cut -d: -f1 || true)"
  action_line="$(grep -n 'ActionInvoked' "$root/scaled-action-signals.log" | head -1 | cut -d: -f1 || true)"
  closed_line="$(grep -n 'NotificationClosed' "$root/scaled-action-signals.log" | head -1 | cut -d: -f1 || true)"
  if [[ "$scaled_action_done" != 1 || -z "$token_value" ||
        -z "$activation_line" || -z "$action_line" || -z "$closed_line" ]] ||
     ! grep -Fq "$token_value" "$root/scaled-action-signals.log" ||
     (( activation_line >= action_line || action_line >= closed_line )); then
    echo "Scaled ${test_scale}x action failed XDG token and ordered signal acceptance"
    cat "$root/scaled-action-signals.log"
    grep -E 'xdg_activation(_token)?_v1@' "$root/posthorn.log" | tail -35 || true
    exit 1
  fi
  kill "$monitor_pid" 2>/dev/null || true
  wait "$monitor_pid" 2>/dev/null || true
  monitor_pid=""
  grim "$root/scaled-action-after.png"
  /usr/bin/python3 "$root/check-dismissed.py" "$root/scaled-action-after.png" || {
    echo "Scaled ${test_scale}x default action failed to dismiss target popup"
    exit 1
  }
  echo "Native ${test_scale}x left click forwarded compositor XDG activation token: OK"
fi

# The compositor-originated left click must invoke the sender's default
# action and dismiss only that notification. Observe the D-Bus signal.
if [[ "$test_scale" == 1 ]]; then
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" monitor org.posthorn.Development \
    >"$root/action-signal.log" 2>&1 &
  monitor_pid=$!
  sleep 0.2
  action_result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn action fixture' 0 '' 'Click default action' '' \
    2 'default' 'Open' 1 'category' 's' 'warning' 30000)"
  [[ "$action_result" == u\ * ]] || { echo "Invalid action notification result: $action_result"; exit 1; }
  action_ready=0
  for _ in {1..15}; do
    sleep 0.1
    grim "$root/before-action-click.png"
    if /usr/bin/python3 - "$root/before-action-click.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
pixels = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if pixels[(113,85,54)] >= 300 else 1)
PY
    then action_ready=1; break; fi
  done
  [[ "$action_ready" == 1 ]] || { echo 'Action popup never reached compositor'; exit 1; }
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 1100 25 1280 720 272
  action_signal=0
  for _ in {1..30}; do
    if grep -q 'ActionInvoked' "$root/action-signal.log"; then
      action_signal=1
      break
    fi
    sleep 0.1
  done
  if [[ "$action_signal" != 1 ]]; then
    echo 'Compositor left click did not emit Freedesktop ActionInvoked'
    cat "$root/action-signal.log"
    exit 1
  fi
  # In addition to the action signal, observe the compositor's actual
  # XDG activation-token handshake. This tests the asynchronous Done path
  # rather than inferring success from a synthetic action-dispatch token.
  activation_done=0
  for _ in {1..10}; do
    if grep -Eq 'xdg_activation_token_v1@[^ ]+[.]done' "$root/posthorn.log"; then
      activation_done=1
      break
    fi
    sleep 0.1
  done
  if [[ "$activation_done" != 1 ]]; then
    echo 'Action click did not receive compositor XDG activation token Done'
    grep -E 'xdg_activation(_token)?_v1@' "$root/posthorn.log" | tail -35 || true
    exit 1
  fi
  # Require the exact nonempty compositor token to be forwarded through the
  # Freedesktop ActivationToken signal ahead of ActionInvoked/closed.
  token_value="$(sed -nE 's/.*xdg_activation_token_v1@[^ ]+[.]done, [(]Some[(]"([^"]+)"[)].*/\1/p' "$root/posthorn.log" | tail -1)"
  # D-Bus monitoring is asynchronous. Wait for all three signals to reach
  # the monitor rather than racing its output flush after ActionInvoked.
  for _ in {1..20}; do
    if grep -q 'ActivationToken' "$root/action-signal.log" &&
       grep -q 'ActionInvoked' "$root/action-signal.log" &&
       grep -q 'NotificationClosed' "$root/action-signal.log"; then
      break
    fi
    sleep 0.1
  done
  activation_line="$(grep -n 'ActivationToken' "$root/action-signal.log" | head -1 | cut -d: -f1 || true)"
  action_line="$(grep -n 'ActionInvoked' "$root/action-signal.log" | head -1 | cut -d: -f1 || true)"
  closed_line="$(grep -n 'NotificationClosed' "$root/action-signal.log" | head -1 | cut -d: -f1 || true)"
  if [[ -z "$token_value" ]] ||
     ! grep -Fq "$token_value" "$root/action-signal.log" ||
     [[ -z "$activation_line" || -z "$action_line" || -z "$closed_line" ]] ||
     (( activation_line >= action_line || action_line >= closed_line )); then
    echo 'Compositor-issued token was not forwarded before the action and close signals'
    grep -E 'xdg_activation(_token)?_v1@' "$root/posthorn.log" | tail -15 || true
    cat "$root/action-signal.log"
    exit 1
  fi
  echo 'Compositor activation token forwarded ahead of action and close: OK'
  kill "$monitor_pid" 2>/dev/null || true
  wait "$monitor_pid" 2>/dev/null || true
  monitor_pid=""
  grim "$root/after-action-click.png"
  /usr/bin/python3 "$root/check-dismissed.py" "$root/after-action-click.png" || {
    echo 'Compositor default action did not dismiss just the action popup'
    exit 1
  }
  # A click outside the input region and the default unbound middle button
  # must leave both existing popups untouched, over real compositor input.
  # Neither should waste a compositor XDG activation-token request.
  activation_requests_before="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 100 25 1280 720 273
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 1100 25 1280 720 274
  sleep 0.2
  activation_requests_after="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  if [[ "$activation_requests_before" != "$activation_requests_after" ]]; then
    echo 'A non-action click unexpectedly requested a compositor activation token'
    exit 1
  fi
  grim "$root/after-noop-clicks.png"
  /usr/bin/python3 "$root/check-dismissed.py" "$root/after-noop-clicks.png" || {
    echo 'Click outside the popup or unbound middle button unexpectedly dismissed a notification'
    exit 1
  }

  # Mako requests a real activation token for a matching sender action,
  # even when actions=0 prevents the ActivationToken/ActionInvoked signals.
  # The notification must still dismiss. Exercise that precise edge case
  # via real compositor input and a separate private D-Bus monitor.
  cat >> "$root/mako.conf" <<'EOF'
[category=disabled-action]
background-color=#715536
actions=0
EOF
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" monitor org.posthorn.Development \
    >"$root/disabled-action-signals.log" 2>&1 &
  monitor_pid=$!
  sleep 0.2
  disabled_result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn disabled action' 0 '' 'No action signals' '' \
    2 'default' 'Open' 1 'category' 's' 'disabled-action' 30000)"
  [[ "$disabled_result" == u\ * ]] || { echo 'Invalid disabled-action Notify result'; exit 1; }
  disabled_visible=0
  for _ in {1..15}; do
    sleep 0.1
    grim "$root/disabled-action-before.png"
    if /usr/bin/python3 - "$root/disabled-action-before.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
colors = Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if colors[(113,85,54)] >= 300 else 1)
PY
    then disabled_visible=1; break; fi
  done
  [[ "$disabled_visible" == 1 ]] || { echo 'Disabled-action popup was not painted'; exit 1; }
  prior_requests="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  prior_replies="$(grep -Ec 'xdg_activation_token_v1@[^ ]+[.]done' "$root/posthorn.log" || true)"
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 1100 25 1280 720 272
  disabled_closed=0
  for _ in {1..20}; do
    if grep -q 'Member=NotificationClosed' "$root/disabled-action-signals.log"; then
      disabled_closed=1
      break
    fi
    sleep 0.1
  done
  latest_requests="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  latest_replies="$(grep -Ec 'xdg_activation_token_v1@[^ ]+[.]done' "$root/posthorn.log" || true)"
  if [[ "$disabled_closed" != 1 ]] ||
     grep -Eq 'Member=(ActivationToken|ActionInvoked)' "$root/disabled-action-signals.log" ||
     (( latest_requests != prior_requests + 1 || latest_replies != prior_replies + 1 )); then
    echo 'actions=0 did not suppress signals while preserving token request and dismissal'
    cat "$root/disabled-action-signals.log"
    exit 1
  fi
  kill "$monitor_pid" 2>/dev/null || true
  wait "$monitor_pid" 2>/dev/null || true
  monitor_pid=""
  grim "$root/disabled-action-after.png"
  /usr/bin/python3 "$root/check-dismissed.py" "$root/disabled-action-after.png" || {
    echo 'actions=0 left-click failed to remove only the target popup'
    exit 1
  }
fi
# Mako's [output=] criteria use the compositor-assigned wl_surface output,
# not the requested output= placement field. This fixture deliberately has
# no global output= setting; Sway must report HEADLESS-1 via wl_surface.enter.
cat >> "$root/mako.conf" <<'EOF'
[category=output-probe output=HEADLESS-1]
background-color=#256F9C
[category=output-probe output=NONEXISTENT]
background-color=#C51F76
EOF
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
output_probe="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn output-criteria QA' 0 '' 'Compositor output match' '' \
  0 1 'category' 's' 'output-probe' 30000)"
[[ "$output_probe" == u\ * ]] || { echo 'Invalid output-criteria notification'; exit 1; }
output_probe_id="${output_probe#u }"
output_painted=0
for _ in {1..20}; do
  sleep 0.15
  grim "$root/output-criteria.png"
  if /usr/bin/python3 - "$root/output-criteria.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
pixels=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if pixels[(37,111,156)] >= 300
    and pixels[(197,31,118)] < 100 else 1)
PY
  then output_painted=1; break; fi
done
[[ "$output_painted" == 1 ]] || {
  echo 'Compositor-selected HEADLESS-1 did not match its output-scoped style'
  cat "$root/posthorn.log"
  exit 1
}
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications CloseNotification 'u' "$output_probe_id" >/dev/null

# Mako v1.11 criteria.c requires notif->surface for [output].
# A fresh Notify has no surface while its initial timeout is selected,
# even if the compositor mapped another notification to HEADLESS-1.
# After it acquires a surface, an explicit-ID replacement retains that
# association and can select the output-specific short timeout.
if [[ "$test_scale" == 1 ]]; then
  cat >> "$root/mako.conf" <<'EOF'
[category=output-policy output=HEADLESS-1]
background-color=#9E6235
default-timeout=1800
history=0
EOF
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s -- call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn scoped-policy test' 0 '' 'Output-scoped expiry fixture' '' \
    0 1 'category' 's' 'output-policy' -1)"
  [[ "$result" == u\ * ]] || { echo 'Invalid output policy notification ID'; exit 1; }
  policy_seen=0
  for _ in {1..10}; do
    sleep 0.1
    grim "$root/output-policy-visible.png"
    if /usr/bin/python3 - "$root/output-policy-visible.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(158,98,53)] >= 300 else 1)
PY
    then policy_seen=1; break; fi
  done
  [[ "$policy_seen" == 1 ]] || {
    echo 'Output-scoped expiration policy popup never painted'
    exit 1
  }
  # The fresh notification must NOT inherit [output]'s 1800ms timeout.
  sleep 2.0
  grim "$root/output-policy-still-visible.png"
  if ! /usr/bin/python3 - "$root/output-policy-still-visible.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(158,98,53)] >= 300 else 1)
PY
  then
    echo 'Fresh notification incorrectly inherited output-scoped timeout'
    exit 1
  fi
  # A direct replacement reuses the already associated notification surface.
  # Its initial criteria now match HEADLESS-1 and choose the scoped timeout.
  policy_id="${result#u }"
  replaced="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s -- call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn scoped-policy test' "$policy_id" '' 'Output-scoped expiry fixture' '' \
    0 1 'category' 's' 'output-policy' -1)"
  [[ "$replaced" == "u $policy_id" ]] || {
    echo 'Direct replacement failed to preserve output-policy ID'
    exit 1
  }
  sleep 1.8
  policy_expired=0
  for _ in {1..16}; do
    sleep 0.15
    grim "$root/output-policy-expired.png"
    if /usr/bin/python3 - "$root/output-policy-expired.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(158,98,53)] < 100 else 1)
PY
    then policy_expired=1; break; fi
  done
  [[ "$policy_expired" == 1 ]] || { echo 'Output-scoped default timeout did not expire'; exit 1; }
  policy_history="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListHistory)"
  if [[ "$policy_history" == *'Output-scoped expiry fixture'* ]]; then
    echo 'Output-scoped history=0 incorrectly preserved expired notification'
    exit 1
  fi
fi

# If the sender advertises actions but omits "default", Mako's
# invoke-default-action binding still dismisses on left click. It does not
# request XDG activation or emit action signals without a matching key.
if [[ "$test_scale" == 1 ]]; then
  cat >> "$root/mako.conf" <<'EOF'
[category=no-default-action]
background-color=#AF7925
EOF
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" monitor org.posthorn.Development \
    >"$root/missing-action-signals.log" 2>&1 &
  monitor_pid=$!
  sleep 0.2
  missing_result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn missing default QA' 0 '' 'No default action key' '' \
    2 'open' 'Open' 1 'category' 's' 'no-default-action' 30000)"
  [[ "$missing_result" == u\ * ]] || { echo 'Invalid missing-default Notify result'; exit 1; }
  missing_visible=0
  for _ in {1..15}; do
    sleep 0.1
    grim "$root/missing-default-before.png"
    if /usr/bin/python3 - "$root/missing-default-before.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(175,121,37)] >= 300 else 1)
PY
    then missing_visible=1; break; fi
  done
  [[ "$missing_visible" == 1 ]] || { echo 'Missing-default popup never painted'; exit 1; }
  before_tokens="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 1100 25 1280 720 272
  closed_missing=0
  for _ in {1..20}; do
    if grep -q 'Member=NotificationClosed' "$root/missing-action-signals.log"; then
      closed_missing=1
      break
    fi
    sleep 0.1
  done
  after_tokens="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  if [[ "$closed_missing" != 1 ]] ||
     grep -Eq 'Member=(ActivationToken|ActionInvoked)' "$root/missing-action-signals.log" ||
     [[ "$before_tokens" != "$after_tokens" ]]; then
    echo 'Missing-default click must close without compositor token or action signals'
    cat "$root/missing-action-signals.log"
    exit 1
  fi
  kill "$monitor_pid" 2>/dev/null || true
  wait "$monitor_pid" 2>/dev/null || true
  monitor_pid=""
  grim "$root/missing-default-after.png"
  /usr/bin/python3 - "$root/missing-default-after.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
assert counts[(175,121,37)] < 100, "missing-default popup remained after click"
assert counts[(48,87,71)] >= 300, "older success popup disappeared"
assert counts[(104,65,72)] >= 300, "older failure popup disappeared"
PY
fi

# Verify a nondefault configured middle-button binding through real wl_pointer
# delivery. dismiss --no-history must remove the popup and never save it.
if [[ "$test_scale" == 1 ]]; then
  cat >> "$root/mako.conf" <<'EOF'
[category=middle-dismiss]
background-color=#2B8C9B
on-button-middle=dismiss --no-history
EOF
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" monitor org.posthorn.Development \
    >"$root/middle-binding-signals.log" 2>&1 &
  monitor_pid=$!
  sleep 0.2
  middle_result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn middle binding' 0 '' 'Middle button no history' '' \
    0 1 'category' 's' 'middle-dismiss' 30000)"
  [[ "$middle_result" == u\ * ]] || { echo 'Invalid middle-binding Notify result'; exit 1; }
  middle_visible=0
  for _ in {1..15}; do
    sleep 0.1
    grim "$root/middle-before.png"
    if /usr/bin/python3 - "$root/middle-before.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(43,140,155)] >= 300 else 1)
PY
    then middle_visible=1; break; fi
  done
  [[ "$middle_visible" == 1 ]] || { echo 'Custom middle-binding popup never painted'; exit 1; }
  before_tokens="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 1100 25 1280 720 274
  middle_closed=0
  for _ in {1..20}; do
    if grep -q 'Member=NotificationClosed' "$root/middle-binding-signals.log"; then
      middle_closed=1
      break
    fi
    sleep 0.1
  done
  after_tokens="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  if [[ "$middle_closed" != 1 ]] || [[ "$before_tokens" != "$after_tokens" ]] ||
     grep -Eq 'Member=(ActivationToken|ActionInvoked)' "$root/middle-binding-signals.log"; then
    echo 'Custom middle dismiss did not close without action/activation'
    cat "$root/middle-binding-signals.log"
    exit 1
  fi
  kill "$monitor_pid" 2>/dev/null || true
  wait "$monitor_pid" 2>/dev/null || true
  monitor_pid=""
  grim "$root/middle-after.png"
  /usr/bin/python3 - "$root/middle-after.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
assert counts[(43,140,155)] < 100, "middle-binding notification remained visible"
assert counts[(48,87,71)] >= 300 and counts[(104,65,72)] >= 300, \
    "unrelated popups did not survive middle-button dismissal"
PY
  middle_history="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListHistory)"
  [[ "$middle_history" != *'Middle button no history'* ]] || {
    echo 'dismiss --no-history unexpectedly saved the notification'
    exit 1
  }
fi

# One real compositor right-click must dismiss every notification in the
# clicked notification's group, not just the top visible member.
if [[ "$test_scale" == 1 ]]; then
  cat >> "$root/mako.conf" <<'EOF'
[category=group-qa]
background-color=#7657AF
group-by=app-name
on-button-right=dismiss-group
EOF
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" monitor org.posthorn.Development \
    >"$root/group-dismiss-signals.log" 2>&1 &
  monitor_pid=$!
  sleep 0.2
  for title in "Group QA first" "Group QA second"; do
    group_result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      org.posthorn.Development /org/freedesktop/Notifications \
      org.freedesktop.Notifications Notify 'susssasa{sv}i' \
      'Posthorn grouped sender' 0 '' "$title" '' \
      0 1 'category' 's' 'group-qa' 30000)"
    [[ "$group_result" == u\ * ]] || { echo 'Invalid group QA Notify result'; exit 1; }
  done
  active_group="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
  [[ "$active_group" == *'Group QA first'* && "$active_group" == *'Group QA second'* ]] || {
    echo 'Grouped notifications were not both active before clicking'
    exit 1
  }
  group_visible=0
  for _ in {1..15}; do
    sleep 0.1
    grim "$root/group-before.png"
    if /usr/bin/python3 - "$root/group-before.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
raise SystemExit(0 if counts[(118,87,175)] >= 300 else 1)
PY
    then group_visible=1; break; fi
  done
  [[ "$group_visible" == 1 ]] || { echo 'Grouped popup never painted'; exit 1; }
  before_tokens="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  POSTHORN_HEADLESS_POINTER_TEST=1 ./target/debug/examples/qa_virtual_pointer 1100 25 1280 720 273
  group_closed=0
  for _ in {1..20}; do
    closes="$(grep -c 'Member=NotificationClosed' "$root/group-dismiss-signals.log" || true)"
    if (( closes >= 2 )); then group_closed=1; break; fi
    sleep 0.1
  done
  after_tokens="$(grep -Ec 'xdg_activation_v1@[^ ]+[.]get_activation_token' "$root/posthorn.log" || true)"
  if [[ "$group_closed" != 1 || "$before_tokens" != "$after_tokens" ]] ||
     grep -Eq 'Member=(ActivationToken|ActionInvoked)' "$root/group-dismiss-signals.log"; then
    echo 'Grouped right-button binding did not close both members without action tokens'
    cat "$root/group-dismiss-signals.log"
    exit 1
  fi
  kill "$monitor_pid" 2>/dev/null || true
  wait "$monitor_pid" 2>/dev/null || true
  monitor_pid=""
  active_group="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
  if [[ "$active_group" == *'Group QA first'* || "$active_group" == *'Group QA second'* ]]; then
    echo 'Grouped dismiss left at least one matching member active'
    exit 1
  fi
  grim "$root/group-after.png"
  /usr/bin/python3 - "$root/group-after.png" <<'PY'
from collections import Counter
from PIL import Image
import sys
counts=Counter(Image.open(sys.argv[1]).convert("RGB").getdata())
assert counts[(118,87,175)] < 100, "group popup survived dismiss-group"
assert counts[(48,87,71)] >= 300 and counts[(104,65,72)] >= 300, \
    "group dismissal removed unrelated notifications"
PY
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
# Pinned Mako v1.11.0 reset_notification() destroys the previous timer
# when Notify replaces an active ID, then schedules a fresh timer from the
# replacement payload. Verify this against the private D-Bus and live Sway
# popup, not only the deterministic Rust timer unit.
if [[ "$test_scale" == 1 ]]; then
  timer_first="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn timer replacement QA' 0 '' 'Original timeout' '' 0 1 \
    'category' 's' 'warning' 1200)"
  [[ "$timer_first" == u\ * ]] || { echo 'Invalid timer replacement ID'; exit 1; }
  timer_id="${timer_first#u }"
  # Keep the original deadline pending before replacing it. The extended
  # notification must stay live beyond the original 1200ms lifetime.
  sleep 0.2
  timer_second="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn timer replacement QA' "$timer_id" '' 'Extended timeout' '' 0 1 \
    'category' 's' 'warning' 3000)"
  [[ "$timer_second" == "u $timer_id" ]] || {
    echo 'Timed Notify replacement did not retain the original ID'
    exit 1
  }
  sleep 1.35
  timer_live="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
  [[ "$timer_live" == *'Extended timeout'* &&
     "$timer_live" != *'Original timeout'* ]] || {
    echo 'The original expiration timer closed or reverted the replacement'
    echo "$timer_live"
    exit 1
  }
  grim "$root/timer-replaced-visible.png"
  ./target/debug/examples/qa_png_probe \
    "$root/timer-replaced-visible.png" '#715536' 300 present >/dev/null || {
    echo 'Replacement popup disappeared at the original timer deadline'
    exit 1
  }
  timer_closed=0
  for _ in {1..28}; do
    sleep 0.15
    timer_live="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
    if [[ "$timer_live" != *'Extended timeout'* ]]; then
      timer_closed=1
      break
    fi
  done
  [[ "$timer_closed" == 1 ]] || {
    echo 'Replacement deadline did not close the active notification'
    exit 1
  }
  timer_history="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListHistory)"
  [[ "$timer_history" == *'Extended timeout'* &&
     "$timer_history" != *'Original timeout'* ]] || {
    echo 'Expiry history did not retain only the replacement payload'
    exit 1
  }
  echo 'Private compositor timer replacement: old deadline canceled; replacement expired into history OK'
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
# The new rule must be installed before a mode change can activate it.
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
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
# Mako includes an external INI fragment at the exact point encountered.
# Verify a live active popup inherits that file, then repaints after the
# fragment changes; a malformed included file must reject Reload without
# erasing the last valid style. Only the disposable private session is used.
if [[ "$test_scale" == 1 ]]; then
  printf '[category=qa-include]\nbackground-color=#4A872F\n' > "$root/included-colors.conf"
  sed -i "1i include=$root/included-colors.conf" "$root/mako.conf"
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  included_reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn include QA' 0 '' 'Nested fragment popup' '' \
    0 1 'category' 's' 'qa-include' 30000)"
  [[ "$included_reply" == u\ * ]] || { echo 'Unexpected included config Notify reply'; exit 1; }
  included_id="$(printf '%s\n' "$included_reply" | cut -d ' ' -f 2)"
  included_painted=0
  for _ in {1..25}; do
    sleep 0.12
    grim "$root/include-before.png"
    if ./target/debug/examples/qa_png_probe \
        "$root/include-before.png" '#4A872F' 300 present >/dev/null 2>&1; then
      included_painted=1
      break
    fi
  done
  [[ "$included_painted" == 1 ]] || {
    echo 'Notification did not paint the included theme'
    cat "$root/posthorn.log"
    exit 1
  }

  printf '[category=qa-include]\nbackground-color=#7952A3\n' > "$root/included-colors.conf"
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  include_repainted=0
  for _ in {1..25}; do
    sleep 0.12
    grim "$root/include-after.png"
    if ./target/debug/examples/qa_png_probe \
        "$root/include-after.png" '#7952A3' 300 present >/dev/null 2>&1 &&
       ./target/debug/examples/qa_png_probe \
        "$root/include-after.png" '#4A872F' 80 absent >/dev/null 2>&1; then
      include_repainted=1
      break
    fi
  done
  [[ "$include_repainted" == 1 ]] || {
    echo 'Changing an included configuration fragment did not repaint the active popup'
    cat "$root/posthorn.log"
    exit 1
  }

  printf '[category=qa-include]\nbackground-color=invalid-color\n' > "$root/included-colors.conf"
  if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload \
      >"$root/invalid-include.log" 2>&1; then
    echo 'Invalid included fragment reload was incorrectly accepted'
    exit 1
  fi
  grim "$root/include-rejected.png"
  ./target/debug/examples/qa_png_probe \
    "$root/include-rejected.png" '#7952A3' 300 present >/dev/null || {
      echo 'Invalid include erased previously committed active popup style'
      exit 1
    }
  printf '[category=qa-include]\nbackground-color=#7952A3\n' > "$root/included-colors.conf"
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications CloseNotification 'u' "$included_id" >/dev/null
  echo 'Private included fragment reload, live repaint and invalid rollback: OK'
fi

# Mako's configured on-notify exec also launches notification sounds.
# Verify a trusted shell command receives the D-Bus ID and quiet suppresses
# that command, with no physical audio, user bus, or output device involved.
if [[ "$test_scale" == 1 ]]; then
  cat >> "$root/mako.conf" <<EOF
[category=qa-audio-hook]
on-notify=exec printf '%s\n' "\$id" >> '$root/qa-hook.log'
[mode=quiet]
on-notify=none
EOF
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  audible="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn audio-hook QA' 0 '' 'Configured hook executed' '' \
    0 1 'category' 's' 'qa-audio-hook' 30000)"
  [[ "$audible" == u\ * ]] || { echo 'Invalid hook Notify result'; exit 1; }
  audible_id="$(printf '%s\n' "$audible" | cut -d ' ' -f 2)"
  hook_fired=0
  for _ in {1..25}; do
    sleep 0.1
    if [[ -f "$root/qa-hook.log" ]] &&
       [[ "$(cat "$root/qa-hook.log")" == "$audible_id" ]]; then
      hook_fired=1
      break
    fi
  done
  [[ "$hook_fired" == 1 ]] || {
    echo 'Mako-compatible on-notify exec did not receive numeric ID'
    cat "$root/posthorn.log"
    exit 1
  }
  # Style/config reload reapplies appearance, not notification-arrival hooks.
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  sleep 0.25
  [[ "$(cat "$root/qa-hook.log")" == "$audible_id" ]] || {
    echo 'Reload incorrectly replayed a previously delivered sound hook'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications CloseNotification 'u' "$audible_id" >/dev/null
  # Restoring history is not a new sender Notify and must not repeat the
  # configured sound hook. Exercise this through the real control interface.
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako RestoreNotification >/dev/null
  restored="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
  [[ "$restored" == *'Configured hook executed'* ]] || {
    echo 'Sound-hook fixture was not restored through private history control'
    exit 1
  }
  sleep 0.25
  [[ "$(cat "$root/qa-hook.log")" == "$audible_id" ]] || {
    echo 'History restore incorrectly replayed the configured sound hook'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications CloseNotification 'u' "$audible_id" >/dev/null

  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
    's' quiet >/dev/null
  silent="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn silent-hook QA' 0 '' 'No hook should run' '' \
    0 1 'category' 's' 'qa-audio-hook' 30000)"
  [[ "$silent" == u\ * ]] || { echo 'Invalid quiet-hook Notify result'; exit 1; }
  sleep 0.4
  [[ "$(cat "$root/qa-hook.log")" == "$audible_id" ]] || {
    echo 'Quiet mode unexpectedly executed notification sound hook'
    exit 1
  }
  silent_id="$(printf '%s\n' "$silent" | cut -d ' ' -f 2)"
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications CloseNotification 'u' "$silent_id" >/dev/null
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
    's' default >/dev/null
  # Pinned Mako dbus/xdg.c executes on-notify after BOTH fresh Notify and
  # direct replaces_id, using the retained notification ID. Mode changes and
  # style reloads alone must not re-execute it; quiet must suppress replacement
  # hooks until the default mode is restored.
  replace_reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn replacement hook QA' 0 '' 'First arrival' '' \
    0 1 'category' 's' 'qa-audio-hook' 30000)"
  [[ "$replace_reply" == u\ * ]] || { echo 'Invalid replacement fixture ID'; exit 1; }
  replace_id="${replace_reply#u }"
  before_replacement="$(wc -l < "$root/qa-hook.log")"
  # There is no wait on the initial on-notify shell. Ensure it completes
  # before comparing exactly how many later replacements triggered hooks.
  for _ in {1..40}; do
    if [[ "$(tail -n 1 "$root/qa-hook.log")" == "$replace_id" ]]; then
      break
    fi
    sleep 0.1
  done
  [[ "$(tail -n 1 "$root/qa-hook.log")" == "$replace_id" ]] || {
    echo 'First replacement-test notification did not execute its hook'
    exit 1
  }
  before_replacement="$(wc -l < "$root/qa-hook.log")"
  replaced="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn replacement hook QA' "$replace_id" '' 'Same ID replacement' '' \
    0 1 'category' 's' 'qa-audio-hook' 30000)"
  [[ "$replaced" == "u $replace_id" ]] || {
    echo "Direct replacement changed the Freedesktop ID: $replaced"
    exit 1
  }
  normal_replacement_fired=0
  for _ in {1..40}; do
    if [[ "$(wc -l < "$root/qa-hook.log")" -eq $((before_replacement+1)) ]]; then
      normal_replacement_fired=1
      break
    fi
    sleep 0.1
  done
  [[ "$normal_replacement_fired" == 1 ]] || {
    echo 'Direct replacement did not execute its configured on-notify hook'
    exit 1
  }
  [[ "$(tail -n 1 "$root/qa-hook.log")" == "$replace_id" ]] || {
    echo 'Replacement hook did not receive the original notification ID'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
    's' quiet >/dev/null
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn replacement hook QA' "$replace_id" '' 'Quiet replacement' '' \
    0 1 'category' 's' 'qa-audio-hook' 30000 >/dev/null
  sleep 0.3
  [[ "$(wc -l < "$root/qa-hook.log")" -eq $((before_replacement+1)) ]] || {
    echo 'Quiet-mode replacement incorrectly emitted a sound hook'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
    's' default >/dev/null
  sleep 0.2
  [[ "$(wc -l < "$root/qa-hook.log")" -eq $((before_replacement+1)) ]] || {
    echo 'Mode restoration incorrectly replayed a notification hook'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn replacement hook QA' "$replace_id" '' 'Default replacement' '' \
    0 1 'category' 's' 'qa-audio-hook' 30000 >/dev/null
  resumed=0
  for _ in {1..40}; do
    if [[ "$(wc -l < "$root/qa-hook.log")" -eq $((before_replacement+2)) ]]; then
      resumed=1
      break
    fi
    sleep 0.1
  done
  [[ "$resumed" == 1 ]] || {
    echo 'Hook did not resume after quiet mode was cleared'
    exit 1
  }
  [[ "$(tail -n 1 "$root/qa-hook.log")" == "$replace_id" ]] || {
    echo 'Post-quiet replacement hook used the wrong notification ID'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications CloseNotification 'u' "$replace_id" >/dev/null
  echo 'Private on-notify direct replacement/quiet reactivation lifecycle: OK'
  echo 'on-notify ID delivery and quiet-mode sound-hook suppression: OK'
  # Verify all three real outcome categories, rather than only the
  # synthetic qa-audio-hook case. Commands are entirely private log writes.
  cat >> "$root/mako.conf" <<EOF
[category=success]
on-notify=exec printf 'success:%s\n' "\$id" >> '$root/qa-category-hooks.log'
[category=failure]
on-notify=exec printf 'failure:%s\n' "\$id" >> '$root/qa-category-hooks.log'
[category=warning]
on-notify=exec printf 'warning:%s\n' "\$id" >> '$root/qa-category-hooks.log'
[mode=quiet]
on-notify=none
EOF
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  category_ids=""
  category_count=0
  for category in success failure warning; do
    reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      org.posthorn.Development /org/freedesktop/Notifications \
      org.freedesktop.Notifications Notify 'susssasa{sv}i' \
      'Posthorn category-hook QA' 0 '' "Category $category" '' \
      0 1 'category' 's' "$category" 30000)"
    [[ "$reply" == u\ * ]] || { echo "Invalid $category hook ID"; exit 1; }
    note_id="$(printf '%s\n' "$reply" | cut -d ' ' -f 2)"
    category_ids="$category_ids $note_id"
    category_count=$((category_count+1))
    printf '%s:%s\n' "$category" "$note_id" >> "$root/expected-category-hooks.log"
    observed=0
    for _ in {1..40}; do
      if [[ -f "$root/qa-category-hooks.log" ]] &&
         [[ "$(tail -n 1 "$root/qa-category-hooks.log")" == "$category:$note_id" ]]; then
        observed=1
        break
      fi
      sleep 0.1
    done
    [[ "$observed" == 1 ]] || {
      echo "Category $category failed to execute its own hook for ID $note_id"
      exit 1
    }
    [[ "$(wc -l < "$root/qa-category-hooks.log")" -eq "$category_count" ]] || {
      echo "Category $category emitted duplicate hooks"
      exit 1
    }
  done
  cmp -s "$root/expected-category-hooks.log" "$root/qa-category-hooks.log" || {
    echo 'Category hooks did not preserve category names and notification IDs'
    exit 1
  }
  # Reload and mode changes do not represent new sender Notify events.
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  sleep 0.25
  [[ "$(wc -l < "$root/qa-category-hooks.log")" -eq 3 ]] || {
    echo 'Reload replayed category sound hooks'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
    's' quiet >/dev/null
  for category in success failure warning; do
    reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      org.posthorn.Development /org/freedesktop/Notifications \
      org.freedesktop.Notifications Notify 'susssasa{sv}i' \
      'Posthorn quiet-category QA' 0 '' "Quiet $category" '' \
      0 1 'category' 's' "$category" 30000)"
    [[ "$reply" == u\ * ]] || { echo "Invalid quiet $category hook ID"; exit 1; }
    category_ids="$category_ids $(printf '%s\n' "$reply" | cut -d ' ' -f 2)"
  done
  sleep 0.4
  cmp -s "$root/expected-category-hooks.log" "$root/qa-category-hooks.log" || {
    echo 'Quiet mode failed to suppress all three outcome sound hooks'
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako SetMode \
    's' default >/dev/null
  sleep 0.2
  cmp -s "$root/expected-category-hooks.log" "$root/qa-category-hooks.log" || {
    echo 'Leaving quiet mode replayed category sound hooks'
    exit 1
  }
  for note_id in $category_ids; do
    busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      org.posthorn.Development /org/freedesktop/Notifications \
      org.freedesktop.Notifications CloseNotification 'u' "$note_id" >/dev/null
  done
  echo 'Private success/failure/warning hook routing and quiet suppression: OK'
fi

echo "Headless Sway ${test_scale}x + D-Bus popups, compositor pointer at 1x, timer, reload and live modes: OK"

if [[ "$test_scale" == 1 ]]; then
  # Pinned Mako render.c computes max_width only from real visible
  # notifications. A hidden-only overflow cannot map a synthetic canvas.
  # Verify the actual compositor, then expose/hide the SAME private D-Bus
  # notification via Reload. No production notification bus is involved.
  kill "$posthorn_pid"
  wait "$posthorn_pid" 2>/dev/null || true
  posthorn_pid=""
  cat > "$root/zero-visible.conf" <<'EOF_ZERO'
width=100
height=60
margin=4
padding=4
border-size=0
icons=0
max-visible=0
default-timeout=0
background-color=#9361B2
[hidden]
background-color=#9361B2
EOF_ZERO
  grim "$root/zero-visible-baseline.png"
  ./target/debug/posthorn --experimental-private-desktop "$root/zero-visible.conf" \
    >"$root/zero-visible-posthorn.log" 2>&1 &
  posthorn_pid=$!
  ready=0
  for _ in {1..60}; do
    if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
      org.posthorn.Development /org/freedesktop/Notifications >/dev/null 2>&1; then
      ready=1
      break
    fi
    if ! kill -0 "$posthorn_pid" 2>/dev/null; then
      cat "$root/zero-visible-posthorn.log"
      echo 'Zero-visible private daemon stopped before D-Bus registration'
      exit 1
    fi
    sleep 0.15
  done
  [[ "$ready" == 1 ]] || { echo 'Zero-visible private service did not register'; exit 1; }
  reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Posthorn hidden-only QA' 0 '' 'Hidden warning' '' 0 0 30000)"
  [[ "$reply" == u\ * ]] || { echo "Hidden-only Notify failed: $reply"; exit 1; }
  hidden_note_id="${reply#u }"
  sleep 0.4
  grim "$root/zero-visible-hidden.png"
  ./target/debug/examples/qa_png_probe \
    "$root/zero-visible-hidden.png" '#9361B2' 40 restored \
    "$root/zero-visible-baseline.png" >/dev/null || {
      echo 'Hidden-only overflow incorrectly painted a standalone popup'
      cat "$root/zero-visible-posthorn.log"
      exit 1
    }
  sed -i 's/^max-visible=0$/max-visible=1/' "$root/zero-visible.conf"
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  painted=0
  for _ in {1..35}; do
    sleep 0.15
    if grim "$root/zero-visible-now-painted.png" 2>/dev/null &&
      ./target/debug/examples/qa_png_probe \
        "$root/zero-visible-now-painted.png" '#9361B2' 500 increase \
        "$root/zero-visible-baseline.png" >/dev/null 2>&1; then
      painted=1
      break
    fi
  done
  [[ "$painted" == 1 ]] || {
    echo 'Reload max-visible=1 did not reveal the same private notification'
    cat "$root/zero-visible-posthorn.log"
    exit 1
  }
  sed -i 's/^max-visible=1$/max-visible=0/' "$root/zero-visible.conf"
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  hidden_again=0
  for _ in {1..35}; do
    sleep 0.15
    if grim "$root/zero-visible-hidden-again.png" 2>/dev/null &&
      ./target/debug/examples/qa_png_probe \
        "$root/zero-visible-hidden-again.png" '#9361B2' 40 restored \
        "$root/zero-visible-baseline.png" >/dev/null 2>&1; then
      hidden_again=1
      break
    fi
  done
  [[ "$hidden_again" == 1 ]] || {
    echo 'Reload max-visible=0 failed to hide the existing private popup'
    cat "$root/zero-visible-posthorn.log"
    exit 1
  }
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /org/freedesktop/Notifications \
    org.freedesktop.Notifications CloseNotification 'u' "$hidden_note_id" >/dev/null
  echo 'Live Sway hidden-only canvas and reversible max-visible Reload: OK'

  # Real configured-surface width acceptance. Pinned Mako v1.11.0
  # render.c:113-115 clamps synthetic [hidden] notif_width before Pango.
  # The former unconstrained implementation wraps this 64-character
  # label at 260px (~6 lines) instead of the real 84px canvas (~16 lines).
  # This checks actual Sway screenshot height, not just our Rust raster.
  cat > "$root/zero-visible.conf" <<'EOF_HIDDEN_CLAMP'
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
EOF_HIDDEN_CLAMP
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  for title in 'First hidden' 'Second visible'; do
    reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      org.posthorn.Development /org/freedesktop/Notifications \
      org.freedesktop.Notifications Notify 'susssasa{sv}i' \
      'Posthorn narrow-surface QA' 0 '' "$title" '' 0 0 30000)"
    [[ "$reply" == u\ * ]] || {
      echo "Narrow-surface private Notify failed: $reply"
      exit 1
    }
  done
  clamp_verified=0
  bounds=""
  for _ in {1..35}; do
    sleep 0.15
    if grim "$root/hidden-narrow-clamp.png" 2>/dev/null; then
      bounds="$(./target/debug/examples/qa_png_probe \
        "$root/hidden-narrow-clamp.png" '#9361B2' 250 bounds 2>/dev/null || true)"
      if [[ -n "$bounds" ]]; then
        read -r bx0 bx1 by0 by1 <<< "$bounds"
        bound_width=$((bx1-bx0+1))
        bound_height=$((by1-by0+1))
        if (( bound_width <= 84 && bound_height >= 230 )); then
          clamp_verified=1
          break
        fi
      fi
    fi
  done
  [[ "$clamp_verified" == 1 ]] || {
    echo 'Hidden Pango label was not bounded and wrapped by the actual 84px surface'
    echo "Last observed hidden bounds: $bounds"
    cat "$root/zero-visible-posthorn.log"
    exit 1
  }
  echo "Live Sway hidden text constrained by 84px surface: $bound_width x $bound_height OK"
fi

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
  # This rule must not match while the target monitor is absent. After
  # wl_surface.enter on the hotplugged output, the compositor-reported name
  # overrides the ordinary category background on the *same* live popup.
  printf '\n[category=success output=HEADLESS-2]\nbackground-color=#366FBC\n' >> "$root/mako.conf"
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
    'category' 's' 'success' 30000 >/dev/null
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
raise SystemExit(0 if counts[(54, 111, 188)] >= 300
    and counts[(48, 87, 71)] < 100 else 1)
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
