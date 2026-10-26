#!/usr/bin/env bash
# Opt-in real Hyprland QA on a NEW session bus. Never replace the live Mako.
set -euo pipefail

die(){ printf 'Posthorn Hyprland QA: %s\n' "$*" >&2; exit 1; }
script_path="$(cd "$(dirname "$0")" && pwd -P)/$(basename "$0")"
cd "$(dirname "$script_path")/.."

[[ -v POSTHORN_HYPRLAND_QA ]] ||
  die 'explicit POSTHORN_HYPRLAND_QA=1 guard required'
[[ "$POSTHORN_HYPRLAND_QA" == "1" ]] || die 'Hyprland QA not authorized'
# Private QA must not inherit production-name trial arming, even if empty.
for forbidden in POSTHORN_PRODUCTION_TRIAL_AUTHORIZED \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS \
  POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS; do
  [[ ! -v "$forbidden" ]] || die 'private Hyprland QA refuses production-trial environment'
done
if [[ "${1:-}" == '--inside' ]]; then
  [[ "${POSTHORN_QA_NESTED:-}" == "1" && -n "${POSTHORN_QA_PARENT_BUS:-}" ]] ||
    die 'nested QA entrypoint requires original-bus verification by parent'
  [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" &&
     "$DBUS_SESSION_BUS_ADDRESS" != "$POSTHORN_QA_PARENT_BUS" ]] ||
    die 'nested QA must not run on the original session bus'
fi
[[ -v HYPRLAND_INSTANCE_SIGNATURE && -v WAYLAND_DISPLAY && -v XDG_RUNTIME_DIR ]] ||
  die 'a real Hyprland Wayland session is required'
[[ -n "$HYPRLAND_INSTANCE_SIGNATURE" && -n "$WAYLAND_DISPLAY" ]] ||
  die 'missing Hyprland session or Wayland socket'
[[ -d "$XDG_RUNTIME_DIR" ]] || die 'missing XDG_RUNTIME_DIR'
for tool in busctl hyprctl dbus-run-session grim cargo timeout; do
  command -v "$tool" >/dev/null || die "missing prerequisite: $tool"
done
timeout --signal=TERM --kill-after=2s 5s hyprctl monitors >/dev/null ||
  die 'real Hyprland monitor query failed or timed out'

if [[ "$#" != 1 || "$1" != '--inside' ]]; then
  # The original owner, inventory and private child must agree on the
  # same bus, not silently mix the implicit user bus with a supplied URI.
  [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]] ||
    die 'original D-Bus address unavailable; refusing nested test'
  parent_bus_id="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetId)" ||
    die 'could not inspect pinned D-Bus GUID'
  user_bus_id="$(busctl --user --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetId)" ||
    die 'could not inspect original user D-Bus GUID'
  [[ "$parent_bus_id" == s\ * && "$parent_bus_id" == "$user_bus_id" ]] ||
    die 'pinned D-Bus address is not the original user bus'
  owner_before="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    GetNameOwner s org.freedesktop.Notifications 2>/dev/null)" ||
    die 'real notification service has no owner; refusing to test'
  [[ "$owner_before" == s\ * ]] || die 'unexpected original bus owner'
  # Phase A read-only inventory is allowed by this private QA guard.
  # Its safe classification is not permission for a production cutover.
  inventory="$(bash tests/read_only_owner_inventory.sh)" ||
    die 'read-only Mako owner/supervisor inventory refused or failed'
  [[ "$inventory" == *'owner_executable=mako'* &&
     "$inventory" == *'rollback_controller_verified=no'* ]] ||
    die 'Phase A owner inventory returned unexpected content'
  cargo build --quiet --bin posthorn --example qa_png_probe
  # The child gets a new DBUS_SESSION_BUS_ADDRESS but retains WAYLAND_DISPLAY.
  child_status=0
  POSTHORN_QA_NESTED=1 POSTHORN_QA_PARENT_BUS="$DBUS_SESSION_BUS_ADDRESS" \
    POSTHORN_QA_EXPECTED_OWNER="$owner_before" POSTHORN_QA_PARENT_ID="$parent_bus_id" \
    dbus-run-session -- bash "$script_path" --inside || child_status=$?
  # Always re-check the original owner, including when child acceptance fails.
  parent_bus_id_after="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetId)" ||
    die 'original D-Bus daemon vanished during QA'
  [[ "$parent_bus_id_after" == "$parent_bus_id" ]] ||
    die 'original D-Bus daemon changed during QA'
  owner_after="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    GetNameOwner s org.freedesktop.Notifications 2>/dev/null)" ||
    die 'real notification owner vanished during QA'
  [[ "$owner_before" == "$owner_after" ]] ||
    die 'original notification daemon owner changed during QA'
  [[ "$child_status" == 0 ]] ||
    die "nested private Hyprland acceptance failed (exit $child_status); original owner unchanged"
  printf 'PASS: private Hyprland rendering; original notification owner unchanged\n'
  exit 0
fi

[[ -v DBUS_SESSION_BUS_ADDRESS ]] || die 'nested session bus missing'
# Validate the original bus still has the owner checked by the parent.
# This also catches a mistaken nested invocation on an untrusted bus.
owner_in_child="$(busctl --address="$POSTHORN_QA_PARENT_BUS" --timeout=3s call \
  org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
  GetNameOwner s org.freedesktop.Notifications 2>/dev/null)" ||
  die 'original notification service disappeared before private QA'
[[ "$owner_in_child" == "${POSTHORN_QA_EXPECTED_OWNER:-}" ]] ||
  die 'original notification service owner changed before private QA'
# D-Bus addresses can alias the same bus using different URI spellings.
# Compare the actual bus GUIDs too, not only their address strings.
nested_bus_id="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
  org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetId)" ||
  die 'private D-Bus GUID unavailable'
[[ -n "${POSTHORN_QA_PARENT_ID:-}" &&
   "$nested_bus_id" != "$POSTHORN_QA_PARENT_ID" ]] ||
  die 'private QA is still connected to the original D-Bus daemon'
# A private test must never find a standard notifications owner on its
# disposable bus. This is checked again after Posthorn starts to detect
# accidental public-name acquisition by a future implementation change.
assert_no_public_owner(){
  local answer
  answer="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    NameHasOwner s org.freedesktop.Notifications)" ||
    die 'cannot inspect public notification name on private bus'
  [[ "$answer" == 'b false' ]] ||
    die 'standard notification name unexpectedly owned on private QA bus'
}
assert_no_public_owner
root="$(mktemp -d)"
daemon_pid=""
cleanup(){
  if [[ -n "$daemon_pid" ]]; then
    kill "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
  fi
  if [[ -v POSTHORN_KEEP_QA ]] && [[ "$POSTHORN_KEEP_QA" == "1" ]]; then
    printf 'Sensitive private QA screenshots remain at: %s\n' "$root"
  else
    rm -rf -- "$root"
  fi
}
trap cleanup EXIT

cat > "$root/config" <<'EOF'
width=320
height=96
margin=12
padding=8
border-size=0
layer=overlay
anchor=top-right
background-color=#193FA7
text-color=#FFFFFFFF
default-timeout=0
max-visible=3
on-button-left=none
on-button-right=none
on-touch=none
EOF

# Capture a private baseline before the QA layer surface exists.
# The desktop may already contain the chosen color in wallpaper or apps:
# require a *change from that baseline*, not an absolute pixel threshold.
timeout --signal=TERM --kill-after=2s 5s grim "$root/baseline.png" ||
  die 'unable to capture the original Hyprland desktop baseline'

# Use the SAME multi-surface Wayland renderer as the armed trial,
# but only the disposable org.posthorn.Development D-Bus name.
./target/debug/posthorn --experimental-private-multi-desktop "$root/config" \
  >"$root/private-desktop.log" 2>&1 &
daemon_pid=$!
ready=0
for _ in {1..40}; do
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    cat "$root/private-desktop.log" >&2
    die 'private desktop exited before bus name acquisition'
  fi
  response="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    NameHasOwner s org.posthorn.Development 2>/dev/null || true)"
  if [[ "$response" == 'b true' ]]; then ready=1; break; fi
  sleep 0.1
done
[[ "$ready" == 1 ]] || die 'private bus name was not acquired'
assert_no_public_owner
# A different process on the disposable bus must not masquerade as the
# daemon under test. Resolve actual D-Bus ownership to this child PID.
private_unique_owner="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
  org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
  GetNameOwner s org.posthorn.Development)" ||
  die 'cannot resolve private Posthorn D-Bus owner'
[[ "$private_unique_owner" == s\ \"*\" ]] ||
  die 'invalid private owner reply'
private_unique_name="${private_unique_owner#s \"}"
private_unique_name="${private_unique_name%\"}"
private_owner_pid="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
  org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
  GetConnectionUnixProcessID s "$private_unique_name")" ||
  die 'cannot resolve private Posthorn owner PID'
[[ "$private_owner_pid" == "u $daemon_pid" ]] ||
  die 'private Posthorn D-Bus name is not owned by launched daemon PID'


result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn private Hyprland QA' 0 '' 'Temporary QA popup' '' 0 0 30000)"
[[ "$result" == u\ * ]] || die "unexpected Notify reply: $result"
notification_id="$(printf '%s\n' "$result" | awk '{print $2}')"
[[ "$notification_id" =~ ^[1-9][0-9]*$ ]] ||
  die 'private Notify returned an invalid notification ID'
listed="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
[[ "$listed" == *'Temporary QA popup'* ]] ||
  die 'private notification was not active in the control API'

painted=0
for _ in {1..25}; do
  timeout --signal=TERM --kill-after=2s 5s grim "$root/visible.png" ||
    die 'visible popup screenshot timed out'
  if ./target/debug/examples/qa_png_probe \
      "$root/visible.png" '#193FA7' 500 increase "$root/baseline.png" >/dev/null 2>&1; then
    painted=1
    break
  fi
  sleep 0.15
done
if [[ "$painted" != 1 ]]; then
  cat "$root/private-desktop.log" >&2
  die 'real Hyprland screenshot never contained private popup color'
fi

# Reload must restyle the *existing* notification on the real compositor,
# not just future ones. Only the disposable private QA configuration changes.
sed -i 's/background-color=#193FA7/background-color=#307E47/' "$root/config"
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
restyled=0
for _ in {1..25}; do
  timeout --signal=TERM --kill-after=2s 5s grim "$root/reloaded.png" ||
    die 'reloaded popup screenshot timed out'
  if ./target/debug/examples/qa_png_probe \
       "$root/reloaded.png" '#307E47' 500 increase "$root/baseline.png" >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe \
       "$root/reloaded.png" '#193FA7' 60 restored "$root/baseline.png" >/dev/null 2>&1; then
    restyled=1
    break
  fi
  sleep 0.15
done
[[ "$restyled" == 1 ]] ||
  die 'private config reload did not repaint the existing popup on Hyprland'

busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications CloseNotification 'u' "$notification_id" >/dev/null
removed=0
for _ in {1..25}; do
  timeout --signal=TERM --kill-after=2s 5s grim "$root/after-close.png" ||
    die 'closed popup screenshot timed out'
  if ./target/debug/examples/qa_png_probe \
      "$root/after-close.png" '#307E47' 60 restored "$root/baseline.png" >/dev/null 2>&1; then
    removed=1
    break
  fi
  sleep 0.15
done
[[ "$removed" == 1 ]] || die 'closed private popup remained on real Hyprland'
listed_after="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
[[ "$listed_after" != *'Temporary QA popup'* ]] ||
  die 'closed notification remains active on the private control API'

# T503 physical compositor expiry: an actual expiring notification must
# appear, stay on the private name, and vanish after its own timer fires.
# Unlike an immediate CloseNotification, this probes the real
# Wayland notification lifecycle with its event/timer scheduling.
timed_reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn private expiry QA' 0 '' 'Synthetic expiry popup' '' 0 0 9000)"
[[ "$timed_reply" == u\ * ]] ||
  die "private timed Notify returned an unexpected reply: $timed_reply"
timed_visible=0
for _ in {1..40};do
  timeout --signal=TERM --kill-after=2s 5s grim "$root/timed-visible.png" ||
    die 'timed popup screenshot failed'
  if ./target/debug/examples/qa_png_probe \
      "$root/timed-visible.png" '#307E47' 500 increase "$root/baseline.png" >/dev/null 2>&1;then
    timed_visible=1;break
  fi
  sleep 0.15
done
[[ "$timed_visible" == 1 ]] ||
  die 'timed private notification never appeared on real Hyprland'
timed_gone=0
for _ in {1..85};do
  timeout --signal=TERM --kill-after=2s 5s grim "$root/timed-after-expiry.png" ||
    die 'expiry screenshot failed'
  if ./target/debug/examples/qa_png_probe \
      "$root/timed-after-expiry.png" '#307E47' 60 restored "$root/baseline.png" >/dev/null 2>&1;then
    timed_gone=1;break
  fi
  sleep 0.15
done
[[ "$timed_gone" == 1 ]] ||
  die 'private notification did not expire from the real compositor'
after_expiry="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /fr/emersion/Mako fr.emersion.Mako ListNotifications)"
[[ "$after_expiry" != *'Synthetic expiry popup'* ]] ||
  die 'expired notification remains active in the private D-Bus control API'
assert_no_public_owner
printf 'PASS: private Hyprland popup appeared, restyled, closed, and expired on screenshots and control bus\n'
