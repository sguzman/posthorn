#!/usr/bin/env bash
# Isolated upstream/Posthorn geometry differential. No user desktop/bus.
set -euo pipefail
[[ "$POSTHORN_PINNED_DIFFERENTIAL" == 1 ]] || { echo 'explicit guard required'; exit 1; }
[[ -v DBUS_SESSION_BUS_ADDRESS ]] || { echo 'private D-Bus required'; exit 1; }
[[ ! -v WAYLAND_DISPLAY && ! -v HYPRLAND_INSTANCE_SIGNATURE ]] || {
  echo 'refuse existing Wayland desktop'; exit 1;
}
mako_binary="$1"
[[ -x "$mako_binary" ]] || { echo 'pinned Mako binary missing'; exit 1; }
root="$(mktemp -d)"
sway_pid=""
daemon_pid=""
cleanup() {
  if [[ -n "$daemon_pid" ]]; then kill "$daemon_pid" 2>/dev/null || true; wait "$daemon_pid" 2>/dev/null || true; fi
  if [[ -n "$sway_pid" ]]; then kill "$sway_pid" 2>/dev/null || true; wait "$sway_pid" 2>/dev/null || true; fi
  rm -rf "$root"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$root/runtime" WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1
export WLR_RENDERER=pixman XDG_CURRENT_DESKTOP=sway
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
printf 'xwayland disable\noutput * resolution 1280x720 scale 1\n' >"$root/sway.conf"
sway --unsupported-gpu -c "$root/sway.conf" >"$root/sway.log" 2>&1 &
sway_pid=$!
socket=""
for _ in {1..60}; do
  for f in "$XDG_RUNTIME_DIR"/wayland-*; do
    if [[ -S "$f" ]]; then socket="$f"; break; fi
  done
  [[ -n "$socket" ]] && break
  if ! kill -0 "$sway_pid" 2>/dev/null; then cat "$root/sway.log"; exit 1; fi
  sleep 0.2
done
[[ -n "$socket" ]] || { cat "$root/sway.log"; exit 1; }
export WAYLAND_DISPLAY="$(basename "$socket")"
ready=0
for _ in {1..60}; do
  ipc="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
  if [[ -S "$ipc" ]] && swaymsg -s "$ipc" -t get_outputs >/dev/null 2>&1; then
    ready=1; break
  fi
  sleep 0.2
done
[[ "$ready" == 1 ]] || { cat "$root/sway.log"; exit 1; }
cat >"$root/mako.conf" <<'EOF_CONFIG'
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
EOF_CONFIG
grim "$root/baseline.png"
./target/debug/examples/qa_png_probe "$root/baseline.png" '#9361B2' 1 absent >/dev/null
wait_for_bus() {
  local name="$1"
  for _ in {1..60}; do
    if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=2s introspect \
      "$name" /org/freedesktop/Notifications >/dev/null 2>&1; then return 0; fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then cat "$root/daemon.log"; return 1; fi
    sleep 0.15
  done
  echo "service $name did not register"; cat "$root/daemon.log"; return 1
}
notify_pair() {
  local name="$1" title response
  for title in 'First hidden' 'Second visible'; do
    response="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
      "$name" /org/freedesktop/Notifications \
      org.freedesktop.Notifications Notify 'susssasa{sv}i' \
      'Pinned Mako geometry differential' 0 '' "$title" '' 0 0 30000)"
    [[ "$response" == u\ * ]] || { echo "Notify failed: $response"; return 1; }
  done
}
capture_bounds() {
  local label="$1" bounds=""
  for _ in {1..45}; do
    if grim "$root/$label.png" 2>/dev/null; then
      bounds="$(./target/debug/examples/qa_png_probe \
        "$root/$label.png" '#9361B2' 250 bounds 2>/dev/null || true)"
      if [[ -n "$bounds" ]]; then echo "$bounds"; return 0; fi
    fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then cat "$root/daemon.log" >&2; return 1; fi
    sleep 0.15
  done
  echo "missing hidden label for $label" >&2
  cat "$root/daemon.log" >&2
  return 1
}
./target/debug/posthorn --experimental-private-desktop "$root/mako.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
notify_pair org.posthorn.Development
post_bounds="$(capture_bounds posthorn)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
cleared=0
for _ in {1..50}; do
  sleep 0.1
  if grim "$root/cleared.png" 2>/dev/null &&
    ./target/debug/examples/qa_png_probe "$root/cleared.png" '#9361B2' 1 absent \
      >/dev/null 2>&1; then cleared=1; break; fi
done
[[ "$cleared" == 1 ]] || { echo 'Posthorn surface was not removed'; exit 1; }
"$mako_binary" -c "$root/mako.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
notify_pair org.freedesktop.Notifications
mako_bounds="$(capture_bounds mako)"
read -r px0 px1 py0 py1 <<< "$post_bounds"
read -r mx0 mx1 my0 my1 <<< "$mako_bounds"
pw=$((px1-px0+1)); ph=$((py1-py0+1))
mw=$((mx1-mx0+1)); mh=$((my1-my0+1))
echo "Posthorn hidden bounds: $post_bounds ($pw x $ph)"
echo "Pinned Mako hidden bounds: $mako_bounds ($mw x $mh)"
(( pw<=84 && mw<=84 && pw>=70 && mw>=70 && ph>=100 && mh>=100 )) || {
  echo 'invalid or unexpectedly small comparison regions'; exit 1;
}
(( (pw-mw)<=2 && (mw-pw)<=2 && (ph-mh)<=2 && (mh-ph)<=2 )) || {
  echo 'Source-pinned Mako/Posthorn hidden-label geometry differs'; exit 1;
}
echo 'Source-pinned Mako/Posthorn hidden-label geometry: MATCH'

# Pinned upstream Mako's max-visible=0 must also keep its daemon
# alive and its hidden-only label absent. This is an executable
# comparison with the already-accepted private Posthorn zero-visible
# fixture, NOT an inference from render.c's reported width=0.
sed -i 's/^max-visible=1$/max-visible=0/' "$root/mako.conf"
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
  org.freedesktop.Notifications /fr/emersion/Mako fr.emersion.Mako Reload \
  >"$root/hidden-zero-reload.log" 2>&1 || {
  cat "$root/hidden-zero-reload.log"
  cat "$root/daemon.log"
  echo 'Pinned upstream Mako rejected max-visible=0 reload'
  exit 1
}
absent=0
for _ in {1..35}; do
  sleep 0.15
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    echo 'Pinned Mako exited after max-visible=0 reload'
    cat "$root/daemon.log"; exit 1
  fi
  if grim "$root/hidden-zero.png" 2>/dev/null &&
    ./target/debug/examples/qa_png_probe \
      "$root/hidden-zero.png" '#9361B2' 1 absent >/dev/null 2>&1; then
    absent=1; break
  fi
done
[[ "$absent" == 1 ]] || {
  echo 'Pinned Mako unexpectedly painted a hidden-only label'
  cat "$root/daemon.log"; exit 1
}
busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s introspect \
  org.freedesktop.Notifications /org/freedesktop/Notifications >/dev/null
echo 'Pinned upstream max-visible=0: daemon alive, bus responsive, hidden-only pixels absent'

# Cross-binary T205 Pango/format comparison. Both daemons must use
# Mako's summary/body format string with literal XML characters in the
# summary and valid markup within the body; rendered geometry and text
# coverage are compared on the SAME Sway output and font installation.
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
cat >"$root/markup.conf" <<'EOF_MARKUP'
width=180
height=200
font=Sans 17
margin=0
padding=4
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%s\n%b
markup=1
background-color=#416D98
text-color=#E5C58B
EOF_MARKUP
clear_color(){
  local color="$1" cleared=0
  for _ in {1..50}; do
    if grim "$root/markup-cleared.png" 2>/dev/null &&
      ./target/debug/examples/qa_png_probe \
        "$root/markup-cleared.png" "$color" 1 absent >/dev/null 2>&1; then
      cleared=1; break
    fi
    sleep 0.1
  done
  [[ "$cleared" == 1 ]] || { echo "popup $color persisted after daemon exit"; exit 1; }
}
markup_measure(){
  local label="$1" name="$2" bounds="" count_line="" text_pixels=""
  local image="$root/$label-markup.png"
  for _ in {1..45}; do
    if grim "$image" 2>/dev/null; then
      bounds="$(./target/debug/examples/qa_png_probe \
        "$image" '#416D98' 400 bounds 2>/dev/null || true)"
      count_line="$(./target/debug/examples/qa_png_probe \
        "$image" '#E5C58B' 20 present 2>/dev/null || true)"
      if [[ -n "$bounds" && -n "$count_line" ]]; then
        text_pixels="${count_line#matching pixels: }"
        text_pixels="${text_pixels%%;*}"
        echo "$bounds $text_pixels"
        return 0
      fi
    fi
    if ! kill -0 "$daemon_pid" 2>/dev/null; then cat "$root/daemon.log" >&2; return 1; fi
    sleep 0.15
  done
  echo "missing or malformed markup output: $label" >&2
  cat "$root/daemon.log" >&2
  return 1
}
markup_notify(){
  local name="$1" response
  local body='<b>Bold &amp; ready</b> with <i>italic words</i> and <u>underlined detail</u>'
  if (( $#>=2 )); then body="$2"; fi
  response="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    "$name" /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Pinned Mako markup differential' 0 '' \
    'Fish & Chips <ready>' \
    "$body" \
    0 0 30000)"
  [[ "$response" == u\ * ]] || { echo "markup Notify failed: $response"; exit 1; }
}
clear_color '#416D98'
./target/debug/posthorn --experimental-private-desktop "$root/markup.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
markup_notify org.posthorn.Development
post_markup="$(markup_measure posthorn org.posthorn.Development)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
"$mako_binary" -c "$root/markup.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
markup_notify org.freedesktop.Notifications
mako_markup="$(markup_measure mako org.freedesktop.Notifications)"
read -r p0 p1 p2 p3 pc <<< "$post_markup"
read -r m0 m1 m2 m3 mc <<< "$mako_markup"
pw=$((p1-p0+1)); ph=$((p3-p2+1))
mw=$((m1-m0+1)); mh=$((m3-m2+1))
echo "Posthorn Pango markup: background $pw x $ph; text pixels $pc"
echo "Mako v1.11.0 Pango markup: background $mw x $mh; text pixels $mc"
(( pw>=160 && mw>=160 && ph>=40 && mh>=40 && pc>=20 && mc>=20 )) || {
  echo 'Markup layout or text unexpectedly absent'; exit 1;
}
(( (pw-mw)<=2 && (mw-pw)<=2 && (ph-mh)<=2 && (mh-ph)<=2 )) || {
  echo 'Pinned Mako/Posthorn Pango format geometry differs'; exit 1;
}
pixel_slack=$((mc/20+20))
(( (pc-mc)<=pixel_slack && (mc-pc)<=pixel_slack )) || {
  echo "Pinned Mako/Posthorn text coverage differs by more than $pixel_slack pixels"
  exit 1
}
echo 'Pinned Mako/Posthorn Pango markup and format geometry: MATCH'

# The same pinned source also requires markup=0 to escape sender-provided
# body tags rather than interpreting them as bold/italic/underline. Compare
# actual text coverage under both executables, not a Posthorn-only fixture.
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
sed -i 's/^markup=1$/markup=0/' "$root/markup.conf"
./target/debug/posthorn --experimental-private-desktop "$root/markup.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
markup_notify org.posthorn.Development
post_literal="$(markup_measure posthorn-literal org.posthorn.Development)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
"$mako_binary" -c "$root/markup.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
markup_notify org.freedesktop.Notifications
mako_literal="$(markup_measure mako-literal org.freedesktop.Notifications)"
read -r lp0 lp1 lp2 lp3 lpc <<< "$post_literal"
read -r lm0 lm1 lm2 lm3 lmc <<< "$mako_literal"
lpw=$((lp1-lp0+1)); lph=$((lp3-lp2+1))
lmw=$((lm1-lm0+1)); lmh=$((lm3-lm2+1))
echo "Posthorn markup-disabled literal body: background $lpw x $lph; text pixels $lpc"
echo "Mako v1.11.0 markup-disabled literal body: background $lmw x $lmh; text pixels $lmc"
(( lpw>=160 && lmw>=160 && lph>=40 && lmh>=40 && lpc>=20 && lmc>=20 )) || {
  echo 'Markup-disabled text or layout unexpectedly absent'; exit 1;
}
(( (lpw-lmw)<=2 && (lmw-lpw)<=2 && (lph-lmh)<=2 && (lmh-lph)<=2 )) || {
  echo 'Pinned Mako/Posthorn literal-body geometry differs'; exit 1;
}
literal_slack=$((lmc/20+20))
(( (lpc-lmc)<=literal_slack && (lmc-lpc)<=literal_slack )) || {
  echo "Pinned Mako/Posthorn literal-body text coverage differs by more than $literal_slack pixels"
  exit 1
}
[[ "$lpc" != "$pc" && "$lmc" != "$mc" ]] || {
  echo 'markup=0 did not materially change literal-tag text coverage'; exit 1
}
echo 'Pinned Mako/Posthorn markup-disabled literal-body format: MATCH'

# A malformed sender body is not trusted Pango markup. Upstream
# notification.c:324-336 escapes invalid body tags; Posthorn must make
# the same fallback choice without discarding the template or summary.
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
sed -i 's/^markup=0$/markup=1/' "$root/markup.conf"
malformed='<b>broken <i>markup without closing tags & raw'
./target/debug/posthorn --experimental-private-desktop "$root/markup.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
markup_notify org.posthorn.Development "$malformed"
post_malformed="$(markup_measure posthorn-malformed org.posthorn.Development)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
"$mako_binary" -c "$root/markup.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
markup_notify org.freedesktop.Notifications "$malformed"
mako_malformed="$(markup_measure mako-malformed org.freedesktop.Notifications)"
read -r xp0 xp1 yp0 yp1 xpc <<< "$post_malformed"
read -r xm0 xm1 ym0 ym1 xmc <<< "$mako_malformed"
xpw=$((xp1-xp0+1)); xph=$((yp1-yp0+1))
xmw=$((xm1-xm0+1)); xmh=$((ym1-ym0+1))
echo "Posthorn malformed-body escaped Pango: background $xpw x $xph; text pixels $xpc"
echo "Mako v1.11.0 malformed-body escaped Pango: background $xmw x $xmh; text pixels $xmc"
(( xpw>=160 && xmw>=160 && xph>=40 && xmh>=40 && xpc>=20 && xmc>=20 )) || {
  echo 'Malformed-body fallback text not painted'; exit 1;
}
(( (xpw-xmw)<=2 && (xmw-xpw)<=2 && (xph-xmh)<=2 && (xmh-xph)<=2 )) || {
  echo 'Mako/Posthorn malformed markup layout differs'; exit 1;
}
fallback_slack=$((xmc/20+20))
(( (xpc-xmc)<=fallback_slack && (xmc-xpc)<=fallback_slack )) || {
  echo "Mako/Posthorn malformed markup coverage differs by more than $fallback_slack pixels"
  exit 1
}
echo 'Pinned Mako/Posthorn malformed sender markup fallback: MATCH'

# Compare the existing T205 Pango fixture at an actual 2x compositor
# output scale, not a post-hoc enlarged screenshot. In pinned Mako,
# render.c scales Pango attributes and layout metrics in physical pixels.
# Both binaries run sequentially on the SAME 2x Sway output.
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
swaymsg -s "$ipc" 'output HEADLESS-1 scale 2' >/dev/null
scaled_ready=0
for _ in {1..60}; do
  if swaymsg -s "$ipc" -t get_outputs >"$root/two-x-outputs.json" &&
      grep -Eq '"scale":[[:space:]]*2([.]0+)?[,}]' "$root/two-x-outputs.json"; then
    scaled_ready=1; break
  fi
  sleep 0.15
done
[[ "$scaled_ready" == 1 ]] || {
  echo 'Private compositor did not apply scale 2'
  cat "$root/two-x-outputs.json"
  exit 1
}
./target/debug/posthorn --experimental-private-desktop "$root/markup.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
markup_notify org.posthorn.Development
post_two_x="$(markup_measure posthorn-two-x org.posthorn.Development)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
"$mako_binary" -c "$root/markup.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
markup_notify org.freedesktop.Notifications
mako_two_x="$(markup_measure mako-two-x org.freedesktop.Notifications)"
read -r tp0 tp1 tp2 tp3 tpc <<< "$post_two_x"
read -r tm0 tm1 tm2 tm3 tmc <<< "$mako_two_x"
tpw=$((tp1-tp0+1)); tph=$((tp3-tp2+1))
tmw=$((tm1-tm0+1)); tmh=$((tm3-tm2+1))
echo "Posthorn actual 2x Pango: background $tpw x $tph; text pixels $tpc"
echo "Mako v1.11.0 actual 2x Pango: background $tmw x $tmh; text pixels $tmc"
(( tpw>=320 && tmw>=320 && tph>=80 && tmh>=80 && tpc>=50 && tmc>=50 )) || {
  echo 'Actual compositor 2x output did not paint full popup or text'
  exit 1
}
(( (tpw-tmw)<=2 && (tmw-tpw)<=2 && (tph-tmh)<=2 && (tmh-tph)<=2 )) || {
  echo 'Pinned Mako/Posthorn actual 2x Pango geometry differs'
  exit 1
}
# Use the same narrow cross-executable tolerance as the 1x fixture;
# print both raw pixel counts before the assertion for diagnosis.
two_x_slack=$((tmc/20+20))
(( (tpc-tmc)<=two_x_slack && (tmc-tpc)<=two_x_slack )) || {
  echo "Pinned Mako/Posthorn 2x text coverage differs by more than $two_x_slack pixels"
  exit 1
}
echo 'Pinned Mako/Posthorn actual compositor 2x Pango markup: MATCH'

# T205/T105 text-height cap and rounded-popup expansion. Pinned Mako
# render.c:139-155 uses style->height for the Pango layout even if
# render.c:208-213 subsequently expands the *popup* to fit its corner
# radii. A second, taller Pango layout would reveal extra glyphs.
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
swaymsg -s "$ipc" 'output HEADLESS-1 scale 1' >/dev/null
scale_one=0
for _ in {1..60}; do
  if swaymsg -s "$ipc" -t get_outputs >"$root/cap-outputs.json" &&
    grep -Eq '"scale":[[:space:]]*1([.]0+)?[,}]' "$root/cap-outputs.json"; then
    scale_one=1; break
  fi
  sleep 0.15
done
[[ "$scale_one" == 1 ]] || {
  echo 'Private compositor did not return to scale 1'
  cat "$root/cap-outputs.json"
  exit 1
}
cat >"$root/cap.conf" <<'EOF_CAP'
width=180
height=45
font=Sans 17
margin=0
padding=4
border-size=0
border-radius=55
icons=0
max-visible=1
default-timeout=0
format=%s\n%b
markup=1
background-color=#416D98
text-color=#E5C58B
EOF_CAP
./target/debug/posthorn --experimental-private-desktop "$root/cap.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
markup_notify org.posthorn.Development
post_cap="$(markup_measure posthorn-height-cap org.posthorn.Development)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
"$mako_binary" -c "$root/cap.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
markup_notify org.freedesktop.Notifications
mako_cap="$(markup_measure mako-height-cap org.freedesktop.Notifications)"
read -r cp0 cp1 cp2 cp3 cpc <<< "$post_cap"
read -r cm0 cm1 cm2 cm3 cmc <<< "$mako_cap"
cpw=$((cp1-cp0+1)); cph=$((cp3-cp2+1))
cmw=$((cm1-cm0+1)); cmh=$((cm3-cm2+1))
echo "Posthorn radius-expanded height-cap: background $cpw x $cph; text pixels $cpc"
echo "Mako v1.11.0 radius-expanded height-cap: background $cmw x $cmh; text pixels $cmc"
(( cpw>=160 && cmw>=160 && cph>=100 && cmh>=100 && cpc>=20 && cmc>=20 )) || {
  echo 'The cap test did not exercise rounded popup expansion with actual text'
  exit 1
}
(( (cpw-cmw)<=2 && (cmw-cpw)<=2 && (cph-cmh)<=2 && (cmh-cph)<=2 )) || {
  echo 'Pinned Mako/Posthorn radius-expanded height-cap geometry differs'
  exit 1
}
[[ "$cpc" == "$cmc" ]] || {
  echo 'Pinned Mako/Posthorn configured text-height cap has different glyph coverage'
  exit 1
}
echo 'Pinned Mako/Posthorn radius-expanded configured Pango height-cap: MATCH'
