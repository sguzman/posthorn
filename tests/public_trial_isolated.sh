#!/usr/bin/env bash
# CI-only standard-name trial proof. Runs solely under a disposable
# dbus-run-session and software Sway; never on the user's real desktop.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == "true" && "${POSTHORN_PUBLIC_TRIAL_CI:-}" == "1" ]] || {
  echo 'Public-name trial acceptance is CI-only and explicitly guarded' >&2
  exit 1
}
[[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" && -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]] || {
  echo 'A disposable CI D-Bus session, not physical Hyprland, is required' >&2
  exit 1
}
root="$(mktemp -d)"
sway_pid=""
trial_pid=""
cleanup(){
  if [[ -n "$trial_pid" ]]; then kill "$trial_pid" 2>/dev/null || true; wait "$trial_pid" 2>/dev/null || true; fi
  if [[ -n "$sway_pid" ]]; then kill "$sway_pid" 2>/dev/null || true; wait "$sway_pid" 2>/dev/null || true; fi
  rm -rf -- "$root"
}
trap cleanup EXIT
cat >"$root/config" <<'EOF_CONF'
width=190
height=90
margin=8
padding=5
border-size=0
background-color=#6F42A8
text-color=#FFFFFF
default-timeout=0
max-visible=3
on-button-left=none
on-button-right=none
on-touch=none
EOF_CONF
# A request without authorization must fail *before* trying to use Wayland
# or claiming a bus name.
if env -u POSTHORN_PRODUCTION_TRIAL_AUTHORIZED \
  -u POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS \
  ./target/debug/posthorn --reversible-production-trial -c "$root/config" \
  >"$root/unarmed.log" 2>&1;then
  echo 'Unarmed public-name trial incorrectly succeeded' >&2;exit 1
fi
grep -q 'explicit authorization required' "$root/unarmed.log"
if POSTHORN_PRODUCTION_TRIAL_AUTHORIZED=1 POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS=60 \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS=unix:path=/wrong-socket \
  ./target/debug/posthorn --reversible-production-trial -c "$root/config" \
  >"$root/wrong-bus.log" 2>&1;then
  echo 'Mismatched public-name bus pin was accepted' >&2;exit 1
fi
grep -q 'session bus does not match controller pin' "$root/wrong-bus.log"
if POSTHORN_PRODUCTION_TRIAL_AUTHORIZED=1 POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS=60 POSTHORN_HYPRLAND_QA=1 \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  ./target/debug/posthorn --reversible-production-trial -c "$root/config" \
  >"$root/private-qa.log" 2>&1;then
  echo 'Private Hyprland QA bypassed the public-name gate' >&2;exit 1
fi
grep -q 'private QA cannot own the public name' "$root/private-qa.log"
# Even with an authorized, correctly pinned bus, an unbounded trial
# must refuse before touching the compositor or acquiring D-Bus names.
if env -u POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS \
  POSTHORN_PRODUCTION_TRIAL_AUTHORIZED=1 \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  ./target/debug/posthorn --reversible-production-trial -c "$root/config" \
  >"$root/unbounded.log" 2>&1;then
  echo 'Unbounded trial unexpectedly passed its lifetime guard' >&2;exit 1
fi
grep -q 'bounded trial lifetime' "$root/unbounded.log"

export XDG_RUNTIME_DIR="$root/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 WLR_RENDERER=pixman XDG_CURRENT_DESKTOP=sway
printf 'xwayland disable\noutput * resolution 1280x720 scale 1\n' >"$root/sway.conf"
sway --unsupported-gpu -c "$root/sway.conf" >"$root/sway.log" 2>&1 &
sway_pid=$!
socket=""
for _ in {1..65};do
  for candidate in "$XDG_RUNTIME_DIR"/wayland-*;do
    [[ ! -S "$candidate" ]] || { socket="$candidate";break; }
  done
  [[ -z "$socket" ]] || break
  kill -0 "$sway_pid" 2>/dev/null || { cat "$root/sway.log" >&2;exit 1; }
  sleep 0.15
done
[[ -n "$socket" ]] || { cat "$root/sway.log" >&2;exit 1; }
export WAYLAND_DISPLAY="${socket##*/}"
ready=0
for _ in {1..50};do
  if timeout 4s grim "$root/baseline.png" >/dev/null 2>&1;then
    ready=1;break
  fi
  sleep 0.15
done
[[ "$ready" == 1 ]] || { echo 'Software compositor capture failed' >&2;exit 1; }
name_has_owner(){
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
    NameHasOwner s "$1"
}
[[ "$(name_has_owner org.freedesktop.Notifications)" == 'b false' ]] || {
  echo 'CI bus already has a notification daemon' >&2;exit 1
}
POSTHORN_PRODUCTION_TRIAL_AUTHORIZED=1 POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS=60 \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  ./target/debug/posthorn --reversible-production-trial -c "$root/config" \
  >"$root/trial.log" 2>&1 &
trial_pid=$!
ready=0
for _ in {1..55};do
  if [[ "$(name_has_owner org.freedesktop.Notifications)" == 'b true' ]];then
    ready=1;break
  fi
  kill -0 "$trial_pid" 2>/dev/null || { cat "$root/trial.log" >&2;exit 1; }
  sleep 0.12
done
[[ "$ready" == 1 ]] || { echo 'Armed standard-name trial did not start' >&2;cat "$root/trial.log" >&2;exit 1; }
[[ "$(name_has_owner org.posthorn.Development)" == 'b false' ]] || {
  echo 'Production trial accidentally acquired the private QA name' >&2;exit 1
}
unique="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
  org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
  GetNameOwner s org.freedesktop.Notifications)"
[[ "$unique" == s\ \"*\" ]] || { echo 'Unexpected standard-name owner result' >&2;exit 1; }
owner="${unique#s \"}";owner="${owner%\"}"
owner_pid="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
  org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
  GetConnectionUnixProcessID s "$owner")"
[[ "$owner_pid" == "u $trial_pid" ]] || {
  echo "Standard name owned by wrong process: $owner_pid expected $trial_pid" >&2;exit 1
}
# A second armed process cannot queue behind the first one. It must fail
# immediately, and the first owner must remain the owner.
# Returning a timeout status would indicate a queued/hung contender,
# not a real DoNotQueue refusal. Fail rather than accept it as safety.
contender_status=0
POSTHORN_PRODUCTION_TRIAL_AUTHORIZED=1 POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS=60 \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  timeout 5s ./target/debug/posthorn --reversible-production-trial \
  -c "$root/config" >"$root/contender.log" 2>&1 || contender_status=$?
if [[ "$contender_status" == 0 || "$contender_status" == 124 || "$contender_status" == 137 ]];then
  echo "Second public-name trial did not reject owner collision cleanly (exit $contender_status)" >&2
  cat "$root/contender.log" >&2
  exit 1
fi
grep -q 'Posthorn trial failed:' "$root/contender.log" || {
  echo 'Collision did not fail through standard-name registration' >&2
  cat "$root/contender.log" >&2;exit 1
}
[[ "$(name_has_owner org.freedesktop.Notifications)" == 'b true' ]] || exit 1
same="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=3s call \
  org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus \
  GetNameOwner s org.freedesktop.Notifications)"
[[ "$same" == "$unique" ]] || {
  echo 'Second process displaced the original standard-name owner' >&2;exit 1
}
reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.freedesktop.Notifications /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Isolated Posthorn public trial' 0 '' 'Trial-only popup' '' 0 0 30000)"
[[ "$reply" == u\ * ]] || { echo "Standard-name Notify failed: $reply" >&2;exit 1; }
id="${reply#u }"
painted=0
for _ in {1..40};do
  if timeout 4s grim "$root/visible.png" >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe "$root/visible.png" \
       '#6F42A8' 300 increase "$root/baseline.png" >/dev/null 2>&1;then
    painted=1;break
  fi
  sleep 0.15
done
[[ "$painted" == 1 ]] || { cat "$root/trial.log" >&2;echo 'Standard-name popup did not paint' >&2;exit 1; }
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.freedesktop.Notifications /org/freedesktop/Notifications \
  org.freedesktop.Notifications CloseNotification u "$id" >/dev/null
removed=0
for _ in {1..40};do
  if timeout 4s grim "$root/removed.png" >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe "$root/removed.png" \
       '#6F42A8' 60 restored "$root/baseline.png" >/dev/null 2>&1;then
    removed=1;break
  fi
  sleep 0.15
done
[[ "$removed" == 1 ]] || { echo 'Trial notification remained visible after close' >&2;exit 1; }
# The owner must remain alive after the last notification disappears.
# A successful screen cleanup alone cannot establish daemon lifetime.
if ! kill -0 "$trial_pid" 2>/dev/null;then
  echo 'Trial process exited unexpectedly after the final notification closed' >&2
  cat "$root/trial.log" >&2
  exit 1
fi
kill "$trial_pid";wait "$trial_pid" 2>/dev/null || true;trial_pid=""
released=0
for _ in {1..40};do
  if [[ "$(name_has_owner org.freedesktop.Notifications)" == 'b false' ]];then
    released=1;break
  fi
  sleep 0.1
done
[[ "$released" == 1 ]] || { echo 'Trial failed to release standard name on exit' >&2;exit 1; }
# A separately armed trial MUST release the public bus name at the
# monotonic lifetime boundary even without an external controller kill.
# Keep a live notification present while the deadline expires, so
# automatic shutdown must also remove its compositor layer surface.
POSTHORN_PRODUCTION_TRIAL_AUTHORIZED=1 POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS=8 \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  ./target/debug/posthorn --reversible-production-trial -c "$root/config" \
  >"$root/auto-expire.log" 2>&1 &
trial_pid=$!
acquired=0
for _ in {1..40};do
  if [[ "$(name_has_owner org.freedesktop.Notifications)" == 'b true' ]];then
    acquired=1;break
  fi
  kill -0 "$trial_pid" 2>/dev/null || { cat "$root/auto-expire.log" >&2;exit 1; }
  sleep 0.10
done
[[ "$acquired" == 1 ]] || { echo 'Short trial never acquired its name' >&2;exit 1; }
reply="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.freedesktop.Notifications /org/freedesktop/Notifications \
  org.freedesktop.Notifications Notify 'susssasa{sv}i' \
  'Isolated TTL cleanup' 0 '' 'Active until trial deadline' '' 0 0 30000)"
[[ "$reply" == u\ * ]] || { echo 'TTL trial failed to accept a notification' >&2;exit 1; }
timeout_popup=0
for _ in {1..50};do
  if timeout 4s grim "$root/deadline-active.png" >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe "$root/deadline-active.png" \
       '#6F42A8' 300 increase "$root/baseline.png" >/dev/null 2>&1;then
    timeout_popup=1;break
  fi
  sleep 0.10
done
[[ "$timeout_popup" == 1 ]] || {
  echo 'Timed trial failed to paint its active notification before deadline' >&2
  cat "$root/auto-expire.log" >&2;exit 1
}
expired=0
for _ in {1..140};do
  if [[ "$(name_has_owner org.freedesktop.Notifications)" == 'b false' ]];then
    expired=1;break
  fi
  sleep 0.10
done
[[ "$expired" == 1 ]] || { echo 'Short trial did not release public name on deadline' >&2;cat "$root/auto-expire.log" >&2;exit 1; }
wait "$trial_pid" || { echo 'Deadline trial returned failure' >&2;cat "$root/auto-expire.log" >&2;exit 1; }
trial_pid=""
grep -q 'reversible trial deadline reached' "$root/auto-expire.log" || {
  echo 'Name vanished without expected monotonic trial deadline' >&2
  cat "$root/auto-expire.log" >&2;exit 1
}
cleared=0
for _ in {1..35};do
  if timeout 4s grim "$root/deadline-cleared.png" >/dev/null 2>&1 &&
     ./target/debug/examples/qa_png_probe "$root/deadline-cleared.png" \
       '#6F42A8' 60 restored "$root/baseline.png" >/dev/null 2>&1;then
    cleared=1;break
  fi
  sleep 0.12
done
[[ "$cleared" == 1 ]] || {
  echo 'Expired trial left its live notification visible on Sway' >&2
  cat "$root/auto-expire.log" >&2;exit 1
}
echo 'PASS: bounded public-name trial expires with a painted popup, releases D-Bus, clears Wayland, and refuses collisions'
