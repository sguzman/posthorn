#!/usr/bin/env bash
# Private wlroots/Sway acceptance for compositor input-region pass-through.
# Never connects to the workstation's Wayland socket or real session bus.
set -euo pipefail
root="$(mktemp -d)"
sway_pid=""
probe_pid=""
cleanup(){
  if [[ -n "$probe_pid" ]]; then kill "$probe_pid" 2>/dev/null || true; wait "$probe_pid" 2>/dev/null || true; fi
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
[[ -n "$socket" ]] || { echo 'Private Sway socket unavailable'; cat "$root/sway.log"; exit 1; }
export WAYLAND_DISPLAY="${socket##*/}"
POSTHORN_HEADLESS_INPUT_TEST=1 ./target/debug/examples/qa_input_region \
  "$root/ready" "$root/received" >"$root/probe.log" 2>&1 &
probe_pid=$!
mapped=0
for _ in {1..100}; do
  if [[ -e "$root/ready" ]]; then mapped=1; break; fi
  if ! kill -0 "$probe_pid" 2>/dev/null; then
    cat "$root/probe.log"
    exit 1
  fi
  sleep 0.1
done
[[ "$mapped" == 1 ]] || { echo 'Overlapping Wayland surfaces were not ready'; cat "$root/probe.log"; exit 1; }
# The overlay occupies x=0..159, y=0..99. Only its right-hand half
# (x=80..159) is in its wl_surface input region. A click in its left half
# must be delivered by Sway to the underlying Top-layer surface.
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer 40 40 1280 720 273
delivered_below=0
for _ in {1..30}; do
  if [[ -e "$root/received" ]] && grep -Fxq lower "$root/received"; then
    delivered_below=1
    break
  fi
  if ! kill -0 "$probe_pid" 2>/dev/null; then cat "$root/probe.log"; exit 1; fi
  sleep 0.1
done
[[ "$delivered_below" == 1 ]] || { echo 'Transparent overlay swallowed a pointer event'; cat "$root/probe.log"; exit 1; }
[[ "$(wc -l <"$root/received")" == 1 ]] && [[ "$(cat "$root/received")" == lower ]] ||
  { echo 'Input region delivered first click to the wrong layer'; cat "$root/received"; exit 1; }
# The next click lands inside the overlay input region and must not be
# dispatched to the underlying layer.
POSTHORN_HEADLESS_POINTER_TEST=1 \
  ./target/debug/examples/qa_virtual_pointer 120 40 1280 720 273
delivered_above=0
for _ in {1..30}; do
  if [[ -e "$root/received" ]] &&
    [[ "$(wc -l <"$root/received")" -ge 2 ]]; then
    delivered_above=1
    break
  fi
  if ! kill -0 "$probe_pid" 2>/dev/null; then cat "$root/probe.log"; exit 1; fi
  sleep 0.1
done
[[ "$delivered_above" == 1 ]] || { echo 'Overlay failed to receive in-region click'; cat "$root/probe.log"; exit 1; }
[[ "$(wc -l <"$root/received")" == 2 ]] &&
  [[ "$(sed -n '2p' "$root/received")" == upper ]] ||
  { echo 'In-region click did not target only the top overlay'; cat "$root/received"; exit 1; }
echo 'Compositor input-region pass-through to lower layer and upper hit: OK'
