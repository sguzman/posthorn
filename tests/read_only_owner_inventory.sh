#!/usr/bin/env bash
# Read-only Phase A notification-owner probe. Use only in delegated local QA.
set -euo pipefail
die(){ printf 'Posthorn inventory: %s\n' "$*" >&2; exit 1; }
[[ -v POSTHORN_HYPRLAND_QA && "$POSTHORN_HYPRLAND_QA" == 1 ]] ||
  die 'explicit POSTHORN_HYPRLAND_QA=1 guard required'
for forbidden in POSTHORN_PRODUCTION_TRIAL_AUTHORIZED \
  POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS \
  POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS; do
  [[ ! -v "$forbidden" ]] ||
    die 'refuse inventory while production-trial environment is present'
done
[[ -v HYPRLAND_INSTANCE_SIGNATURE && -v WAYLAND_DISPLAY &&
   -v DBUS_SESSION_BUS_ADDRESS && -v XDG_RUNTIME_DIR ]] ||
  die 'real Hyprland session and original D-Bus address required'
[[ -n "$HYPRLAND_INSTANCE_SIGNATURE" && -n "$WAYLAND_DISPLAY" &&
   -n "$DBUS_SESSION_BUS_ADDRESS" ]] ||
  die 'real Hyprland session and original D-Bus address required'
for cmd in busctl hyprctl timeout readlink systemctl cut tr;do
  command -v "$cmd" >/dev/null || die "missing prerequisite: $cmd"
done
timeout 5s hyprctl monitors >/dev/null || die 'Hyprland monitor query failed'
dbus_call(){
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=4s call \
    org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus "$@"
}
get_owner(){
  local reply owner
  reply="$(dbus_call GetNameOwner s org.freedesktop.Notifications)" ||
    die 'notification name has no owner'
  [[ "$reply" == s\ \"*\" ]] || die 'invalid owner response'
  owner="$(printf '%s\n' "$reply" | cut -d '"' -f 2)"
  [[ "$owner" == :* ]] || die 'invalid unique owner'
  printf '%s\n' "$owner"
}
get_pid(){
  local reply pid
  reply="$(dbus_call GetConnectionUnixProcessID s "$1")" ||
    die 'owner PID unavailable'
  [[ "$reply" == u\ * ]] || die 'invalid owner PID response'
  pid="$(printf '%s\n' "$reply" | cut -d ' ' -f 2)"
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || die 'invalid owner PID'
  printf '%s\n' "$pid"
}
owner="$(get_owner)"
pid="$(get_pid "$owner")"
exe="$(readlink -e "/proc/$pid/exe")" || die 'owner executable disappeared'
[[ "$exe" == */mako ]] ||
  die 'current owner executable is not Mako; no restart assumptions allowed'
[[ -r "/proc/$pid/cgroup" ]] || die 'owner cgroup inaccessible'
unit=none
supervisor=unverified
candidate=""
ambiguous=0
while IFS= read -r record;do
  cgroup="$(printf '%s\n' "$record" | cut -d ':' -f 3-)"
  # Parse cgroup components literally: wildcard characters and spaces
  # must never undergo shell word splitting or pathname expansion.
  IFS='/' read -r -a components <<<"$cgroup"
  for component in "${components[@]}";do
    if [[ "$component" == *.service ]];then
      [[ -z "$candidate" || "$candidate" == "$component" ]] || ambiguous=1
      candidate="$component"
    fi
  done
done <"/proc/$pid/cgroup"
if (( ambiguous == 0 )) && [[ "$candidate" =~ ^[a-zA-Z0-9_.@:-]+\.service$ ]];then
  mainpid="$(systemctl --user show "$candidate" -p MainPID --value 2>/dev/null || true)"
  if [[ "$mainpid" == "$pid" ]];then
    supervisor=systemd-user-service-mainpid-verified
    unit="$candidate"
  fi
fi
[[ "$(get_owner)" == "$owner" && "$(get_pid "$owner")" == "$pid" ]] ||
  die 'notification owner changed during inventory'
printf 'phase=read-only\nexisting_name=org.freedesktop.Notifications\n'
printf 'owner_executable=mako\nowner_pid=%s\n' "$pid"
printf 'supervisor=%s\nsupervisor_unit=%s\n' "$supervisor" "$unit"
printf 'cutover_authorized=no\nrollback_controller_verified=no\n'
printf 'physical_hyprland_qa_complete=no\n'
