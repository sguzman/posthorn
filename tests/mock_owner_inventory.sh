#!/usr/bin/env bash
# Deterministic fixture for read-only Phase A inventory classification.
# Fake session commands + a disposable binary named mako; no real
# desktop, notification bus, user service, or process is modified.
set -euo pipefail
root="$(mktemp -d)"
mako_pid=""
cleanup(){
  if [[ -n "$mako_pid" ]];then
    kill "$mako_pid" 2>/dev/null || true
    wait "$mako_pid" 2>/dev/null || true
  fi
  rm -rf "$root"
}
trap cleanup EXIT
command -v cc >/dev/null || { echo 'C compiler required for mock executable' >&2;exit 1; }
cat >"$root/mako.c" <<'EOF_C'
#include <unistd.h>
int main(void) { for (;;) pause(); }
EOF_C
cc "$root/mako.c" -o "$root/mako"
"$root/mako" &
mako_pid=$!
mkdir -p "$root/bin" "$root/runtime"
cat >"$root/bin/busctl" <<'EOF_BUS'
#!/usr/bin/env bash
set -euo pipefail
case " $* " in
  *" GetNameOwner "*)
    if [[ -v MOCK_OWNER_CHURN ]];then
      n="$(cat "$MOCK_COUNTER")";n=$((n+1));printf '%s\n' "$n" >"$MOCK_COUNTER"
      if (( n > 1 ));then printf 's ":1.89"\n';exit 0;fi
    fi
    printf 's ":1.88"\n'
    ;;
  *" GetConnectionUnixProcessID "*)
    printf 'u %s\n' "$MOCK_OWNER_PID"
    ;;
  *) echo 'Unexpected mock D-Bus operation' >&2;exit 1;;
esac
EOF_BUS
cat >"$root/bin/hyprctl" <<'EOF_HYPR'
#!/usr/bin/env bash
echo 'Monitor test-only'
EOF_HYPR
cat >"$root/bin/systemctl" <<'EOF_SYSTEMD'
#!/usr/bin/env bash
echo 0
EOF_SYSTEMD
chmod +x "$root/bin/busctl" "$root/bin/hyprctl" "$root/bin/systemctl"
export PATH="$root/bin:$PATH"
export POSTHORN_HYPRLAND_QA=1 HYPRLAND_INSTANCE_SIGNATURE=synthetic
export WAYLAND_DISPLAY=wayland-synthetic DBUS_SESSION_BUS_ADDRESS=unix:path=/synthetic
export XDG_RUNTIME_DIR="$root/runtime" MOCK_COUNTER="$root/counter"
unset POSTHORN_PRODUCTION_TRIAL_AUTHORIZED || true
export MOCK_OWNER_PID="$mako_pid"
bash tests/read_only_owner_inventory.sh >"$root/good.txt"
grep -q '^phase=read-only$' "$root/good.txt"
grep -q '^owner_executable=mako$' "$root/good.txt"
grep -q "^owner_pid=$mako_pid$" "$root/good.txt"
grep -q '^supervisor=unverified$' "$root/good.txt"
grep -q '^cutover_authorized=no$' "$root/good.txt"
grep -q '^rollback_controller_verified=no$' "$root/good.txt"
# A different, non-Mako owner must fail safely.
export MOCK_OWNER_PID="$$"
if bash tests/read_only_owner_inventory.sh >"$root/nonmako.txt" 2>&1;then
  echo 'Non-Mako owner incorrectly accepted by read-only inventory' >&2
  exit 1
fi
grep -q 'current owner executable is not Mako' "$root/nonmako.txt"
# A bus-name race must refuse: never accept stale PID/supervisor evidence.
export MOCK_OWNER_PID="$mako_pid" MOCK_OWNER_CHURN=1
printf '0\n' >"$MOCK_COUNTER"
if bash tests/read_only_owner_inventory.sh >"$root/churn.txt" 2>&1;then
  echo 'Bus owner change was incorrectly accepted' >&2
  exit 1
fi
grep -q 'notification owner changed during inventory' "$root/churn.txt"
echo 'PASS: synthetic Mako owner accepted read-only, non-Mako and owner churn rejected'
