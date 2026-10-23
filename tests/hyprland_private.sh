#!/usr/bin/env bash
# Opt-in real Hyprland QA on a NEW session bus. Never replace the live Mako.
set -euo pipefail

die(){ printf 'Posthorn Hyprland QA: %s\n' "$*" >&2; exit 1; }
script_path="$(cd "$(dirname "$0")" && pwd -P)/$(basename "$0")"
cd "$(dirname "$script_path")/.."

[[ -v POSTHORN_HYPRLAND_QA ]] ||
  die 'explicit POSTHORN_HYPRLAND_QA=1 guard required'
[[ "$POSTHORN_HYPRLAND_QA" == "1" ]] || die 'Hyprland QA not authorized'
[[ -v HYPRLAND_INSTANCE_SIGNATURE && -v WAYLAND_DISPLAY && -v XDG_RUNTIME_DIR ]] ||
  die 'a real Hyprland Wayland session is required'
[[ -n "$HYPRLAND_INSTANCE_SIGNATURE" && -n "$WAYLAND_DISPLAY" ]] ||
  die 'missing Hyprland session or Wayland socket'
[[ -d "$XDG_RUNTIME_DIR" ]] || die 'missing XDG_RUNTIME_DIR'
for tool in busctl hyprctl dbus-run-session grim cargo; do
  command -v "$tool" >/dev/null || die "missing prerequisite: $tool"
done
hyprctl monitors >/dev/null || die 'real Hyprland monitor query failed'

if [[ "$#" != 1 || "$1" != '--inside' ]]; then
  # Prove the original session's existing notification daemon keeps ownership.
  owner_before="$(busctl --user --timeout=3s call org.freedesktop.DBus \
    /org/freedesktop/DBus org.freedesktop.DBus GetNameOwner \
    s org.freedesktop.Notifications 2>/dev/null)" ||
    die 'real notification service has no owner; refusing to test'
  [[ "$owner_before" == s\ * ]] || die 'unexpected original bus owner'
  cargo build --quiet --bin posthorn --example qa_png_probe
  # The child gets a new DBUS_SESSION_BUS_ADDRESS but retains WAYLAND_DISPLAY.
  child_status=0
  dbus-run-session -- bash "$script_path" --inside || child_status=$?
  # Always re-check the original owner, including when child acceptance fails.
  owner_after="$(busctl --user --timeout=3s call org.freedesktop.DBus \
    /org/freedesktop/DBus org.freedesktop.DBus GetNameOwner \
    s org.freedesktop.Notifications 2>/dev/null)" ||
    die 'real notification owner vanished during QA'
  [[ "$owner_before" == "$owner_after" ]] ||
    die 'original notification daemon owner changed during QA'
  [[ "$child_status" == 0 ]] ||
    die "nested private Hyprland acceptance failed (exit $child_status); original owner unchanged"
  printf 'PASS: private Hyprland rendering; original notification owner unchanged\n'
  exit 0
fi

[[ -v DBUS_SESSION_BUS_ADDRESS ]] || die 'nested session bus missing'
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
grim "$root/baseline.png" ||
  die 'unable to capture the original Hyprland desktop baseline'

./target/debug/posthorn --experimental-private-desktop "$root/config" \
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

result="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Posthorn private Hyprland QA' 0 '' 'Temporary QA popup' '' 0 0 30000)"
[[ "$result" == u\ * ]] || die "unexpected Notify reply: $result"
notification_id="$(printf '%s\n' "$result" | awk '{print $2}')"

painted=0
for _ in {1..25}; do
  grim "$root/visible.png"
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

busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.posthorn.Development /org/freedesktop/Notifications \
  org.freedesktop.Notifications CloseNotification 'u' "$notification_id" >/dev/null
removed=0
for _ in {1..25}; do
  grim "$root/after-close.png"
  if ./target/debug/examples/qa_png_probe \
      "$root/after-close.png" '#193FA7' 60 restored "$root/baseline.png" >/dev/null 2>&1; then
    removed=1
    break
  fi
  sleep 0.15
done
[[ "$removed" == 1 ]] || die 'closed private popup remained on real Hyprland'
printf 'PASS: private Hyprland popup appeared and disappeared on screenshots\n'
