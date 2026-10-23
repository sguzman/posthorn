#!/usr/bin/env bash
# Automated development-only Wayland acceptance smoke. Never owns the
# desktop org.freedesktop.Notifications name or touches a user's session.
set -euo pipefail

root="$(mktemp -d)"
sway_pid=""
posthorn_pid=""
monitor_pid=""
cleanup() {
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

# Exercise D-Bus and compositor behavior at 1x, 1.25x and 2x outputs.
# The larger outputs keep four notifications visible at once. Fractional
# output smoke does not claim wp_fractional_scale_v1 buffer negotiation.
test_scale="${POSTHORN_TEST_SCALE:-1}"
case "$test_scale" in
  1) resolution="1280x720";;
  1.25|2) resolution="1920x1080";;
  *) echo "Unsupported acceptance test scale: $test_scale"; exit 1;;
esac
# Compile the Rust virtual pointer QA client before sending any
# notifications so its build time cannot consume a test notification TTL.
if [[ "$test_scale" == 1 ]]; then
  cargo build --quiet --example qa_virtual_pointer
fi
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

# After the compositor assigned HEADLESS-1, a new notification's
# output-scoped timeout and history policy must be selected from that real
# monitor. Its sender timeout is -1, so the category's short default wins.
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
echo "Headless Sway ${test_scale}x + D-Bus popups, compositor pointer at 1x, timer, reload and live modes: OK"

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
