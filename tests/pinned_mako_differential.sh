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

# Compare T205 Pango text-alignment against pinned Mako at actual scale 2.
# Pixel *counts* alone do not catch misplaced center/right-aligned glyphs:
# compare the absolute ink bounding rectangles for the same synthetic text.
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
swaymsg -s "$ipc" 'output HEADLESS-1 scale 2' >/dev/null
alignment_scale_ready=0
for _ in {1..60}; do
  if swaymsg -s "$ipc" -t get_outputs >"$root/alignment-outputs.json" &&
    grep -Eq '"scale":[[:space:]]*2([.]0+)?[,}]' "$root/alignment-outputs.json"; then
    alignment_scale_ready=1; break
  fi
  sleep 0.15
done
[[ "$alignment_scale_ready" == 1 ]] || {
  echo 'Private compositor did not return to scale 2 for Pango alignment'
  cat "$root/alignment-outputs.json"
  exit 1
}
cat >"$root/alignment.conf" <<'EOF_ALIGN'
width=180
height=120
font=Sans 17
margin=0
padding=4
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%s
markup=0
text-alignment=center
background-color=#416D98
text-color=#E5C58B
EOF_ALIGN
for align in center right; do
  sed -i "s/^text-alignment=.*/text-alignment=$align/" "$root/alignment.conf"
  clear_color '#416D98'
  ./target/debug/posthorn --experimental-private-desktop "$root/alignment.conf" \
    >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.posthorn.Development
  markup_notify org.posthorn.Development
  post_placement="$(markup_measure "posthorn-align-$align" org.posthorn.Development)"
  post_ink="$(./target/debug/examples/qa_png_probe \
    "$root/posthorn-align-$align-markup.png" '#E5C58B' 20 bounds)"
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  clear_color '#416D98'
  "$mako_binary" -c "$root/alignment.conf" >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.freedesktop.Notifications
  markup_notify org.freedesktop.Notifications
  mako_placement="$(markup_measure "mako-align-$align" org.freedesktop.Notifications)"
  mako_ink="$(./target/debug/examples/qa_png_probe \
    "$root/mako-align-$align-markup.png" '#E5C58B' 20 bounds)"
  echo "Posthorn 2x $align layout+coverage: $post_placement; text bounds: $post_ink"
  echo "Mako 2x $align layout+coverage: $mako_placement; text bounds: $mako_ink"
  [[ "$post_placement" == "$mako_placement" && "$post_ink" == "$mako_ink" ]] || {
    echo "Pinned Mako/Posthorn 2x $align-aligned Pango geometry differs"
    exit 1
  }
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  echo "Pinned Mako/Posthorn physical 2x Pango $align alignment: MATCH"
done

# Source-pinned T205: do not infer scale-2 markup correctness solely
# from earlier scale-1 comparisons. Pango's physical AttrScale can
# change wrapping and the literal/malformed fallback glyph footprint.
# Exercise both branches with the same 2x Sway output and font install.
cat >"$root/markup-two-x.conf" <<'EOF_TWO_X_MARKUP'
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
markup=0
background-color=#416D98
text-color=#E5C58B
EOF_TWO_X_MARKUP
for variant in literal malformed; do
  if [[ "$variant" == malformed ]]; then
    sed -i 's/^markup=0$/markup=1/' "$root/markup-two-x.conf"
  fi
  clear_color '#416D98'
  ./target/debug/posthorn --experimental-private-desktop "$root/markup-two-x.conf" \
    >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.posthorn.Development
  if [[ "$variant" == malformed ]]; then
    markup_notify org.posthorn.Development "$malformed"
  else
    markup_notify org.posthorn.Development
  fi
  post_pixels="$(markup_measure "posthorn-two-x-$variant" org.posthorn.Development)"
  post_ink="$(./target/debug/examples/qa_png_probe \
    "$root/posthorn-two-x-$variant-markup.png" '#E5C58B' 20 bounds)"
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  clear_color '#416D98'
  "$mako_binary" -c "$root/markup-two-x.conf" >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.freedesktop.Notifications
  if [[ "$variant" == malformed ]]; then
    markup_notify org.freedesktop.Notifications "$malformed"
  else
    markup_notify org.freedesktop.Notifications
  fi
  mako_pixels="$(markup_measure "mako-two-x-$variant" org.freedesktop.Notifications)"
  mako_ink="$(./target/debug/examples/qa_png_probe \
    "$root/mako-two-x-$variant-markup.png" '#E5C58B' 20 bounds)"
  echo "Posthorn physical 2x $variant: bounds+pixels $post_pixels; ink $post_ink"
  echo "Pinned Mako physical 2x $variant: bounds+pixels $mako_pixels; ink $mako_ink"
  [[ "$post_pixels" == "$mako_pixels" && "$post_ink" == "$mako_ink" ]] || {
    echo "Mako/Posthorn physical 2x $variant markup fallback differs"
    exit 1
  }
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  echo "Pinned Mako/Posthorn physical 2x $variant markup fallback: MATCH"
done

# Test the complete decoded compositor raster as an independent equality
# oracle. Bounds and color counts can hide glyph antialias changes or
# transposed ink inside the same rectangle. The screenshots are synthetic,
# private CI artifacts and are deleted by the guarded cleanup trap.
for label in posthorn-two-x; do
  ./target/debug/examples/qa_png_probe \
    "$root/$label-markup.png" '#E5C58B' 0 identical \
    "$root/mako-two-x-markup.png"
done
for align in center right; do
  ./target/debug/examples/qa_png_probe \
    "$root/posthorn-align-$align-markup.png" '#E5C58B' 0 identical \
    "$root/mako-align-$align-markup.png"
done
for variant in literal malformed; do
  ./target/debug/examples/qa_png_probe \
    "$root/posthorn-two-x-$variant-markup.png" '#E5C58B' 0 identical \
    "$root/mako-two-x-$variant-markup.png"
done
echo 'Pinned Mako/Posthorn physical 2x decoded entire-screenshot raster equality: MATCH'

# T205: pinned Mako Pango word-character wrapping and final-line
# ellipsization depend on exact physical-scale bounds. These cases
# deliberately use long, XML-escaped text and distinct text-height
# limits while reusing the same 2x compositor and immutable font
# environment. Decoded full-compositor image equality detects both
# displaced glyphs and different ellipsis placement.
cat >"$root/wrapping.conf" <<'EOF_WRAP'
width=154
height=62
font=Sans 17
margin=0
padding=5
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%s\n%b
markup=1
background-color=#416D98
text-color=#E5C58B
EOF_WRAP
wrap_body='<b>Lengthy headline:</b> αβγ delta &amp; epsilon words <i>italic overflow and a deliberately stretchedlongwordwithoutspacesfortestingcharacterwrap</i> followed by more content so the final visible line needs an ellipsis.'
for variant in ellipsized wrapped; do
  if [[ "$variant" == wrapped ]]; then
    sed -i 's/^height=62$/height=238/' "$root/wrapping.conf"
  fi
  clear_color '#416D98'
  ./target/debug/posthorn --experimental-private-desktop "$root/wrapping.conf" \
    >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.posthorn.Development
  markup_notify org.posthorn.Development "$wrap_body"
  post_wrap="$(markup_measure "posthorn-wrap-$variant" org.posthorn.Development)"
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  clear_color '#416D98'
  "$mako_binary" -c "$root/wrapping.conf" >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.freedesktop.Notifications
  markup_notify org.freedesktop.Notifications "$wrap_body"
  mako_wrap="$(markup_measure "mako-wrap-$variant" org.freedesktop.Notifications)"
  echo "Posthorn scale-2 $variant Pango wrapping: $post_wrap"
  echo "Pinned Mako scale-2 $variant Pango wrapping: $mako_wrap"
  [[ "$post_wrap" == "$mako_wrap" ]] || {
    echo "Pinned Mako/Posthorn scale-2 $variant bounds or glyph coverage differ"
    exit 1
  }
  ./target/debug/examples/qa_png_probe \
    "$root/posthorn-wrap-$variant-markup.png" '#E5C58B' 0 identical \
    "$root/mako-wrap-$variant-markup.png"
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  echo "Pinned Mako/Posthorn actual 2x $variant Pango wrapping and ink: MATCH"
done

# T205 fallback at the *final template markup* boundary:
# pinned render.c's pango_parse_markup failure means the fully
# substituted (already-escaped) format is passed to set_text
# literally, not silently reconstructed from summary/body.
# Compare both a valid template control and an unmatched opening
# <b> template on a physical 2x compositor, then require them to
# render differently from each other but exactly like pinned Mako.
cat >"$root/final-template.conf" <<'EOF_FINAL_TEMPLATE'
width=192
height=160
font=Sans 17
margin=0
padding=5
border-size=0
icons=0
max-visible=1
default-timeout=0
markup=1
format=<b>Alert: %s</b>
background-color=#416D98
text-color=#E5C58B
EOF_FINAL_TEMPLATE
valid_template=""
invalid_template=""
for variant in valid invalid; do
  if [[ "$variant" == invalid ]]; then
    sed -i 's|^format=.*$|format=<b>Alert: %s|' "$root/final-template.conf"
  fi
  clear_color '#416D98'
  ./target/debug/posthorn --experimental-private-desktop "$root/final-template.conf" \
    >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.posthorn.Development
  markup_notify org.posthorn.Development
  post_template="$(markup_measure "posthorn-template-$variant" org.posthorn.Development)"
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  clear_color '#416D98'
  "$mako_binary" -c "$root/final-template.conf" >"$root/daemon.log" 2>&1 &
  daemon_pid=$!
  wait_for_bus org.freedesktop.Notifications
  markup_notify org.freedesktop.Notifications
  mako_template="$(markup_measure "mako-template-$variant" org.freedesktop.Notifications)"
  echo "Posthorn scale-2 $variant format-template fallback: $post_template"
  echo "Pinned Mako scale-2 $variant format-template fallback: $mako_template"
  [[ "$post_template" == "$mako_template" ]] || {
    echo "Pinned Mako/Posthorn source-final $variant template Pango geometry differs"
    exit 1
  }
  ./target/debug/examples/qa_png_probe \
    "$root/posthorn-template-$variant-markup.png" '#E5C58B' 0 identical \
    "$root/mako-template-$variant-markup.png"
  if [[ "$variant" == valid ]]; then
    valid_template="$post_template"
  else
    invalid_template="$post_template"
  fi
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
  echo "Pinned Mako/Posthorn physical 2x $variant final-format markup: MATCH"
done
[[ "$valid_template" != "$invalid_template" ]] || {
  echo 'Valid and invalid final format templates unexpectedly paint identically'
  exit 1
}

# Mako notification.c:format_notif_text substitutes %a, %i, %g, %s
# and %b; %% is literal percent. This source-pinned case must not
# silently pass by matching total text ink alone: check the exact
# decoded compositor frame including its C-format substitutions,
# escaped app/summary XML and accepted sender-body Pango markup.
cat >"$root/substitutions.conf" <<'EOF_SUBS'
width=240
height=230
font=Sans 17
margin=0
padding=5
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%a / id=%i / group=%g / 100%%\n%s\n%b
markup=1
background-color=#416D98
text-color=#E5C58B
EOF_SUBS
clear_color '#416D98'
./target/debug/posthorn --experimental-private-desktop "$root/substitutions.conf" \
  >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.posthorn.Development
markup_notify org.posthorn.Development
post_substitutions="$(markup_measure posthorn-substitutions org.posthorn.Development)"
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
"$mako_binary" -c "$root/substitutions.conf" >"$root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_bus org.freedesktop.Notifications
markup_notify org.freedesktop.Notifications
mako_substitutions="$(markup_measure mako-substitutions org.freedesktop.Notifications)"
echo "Posthorn real 2x format placeholders: $post_substitutions"
echo "Pinned Mako real 2x format placeholders: $mako_substitutions"
[[ "$post_substitutions" == "$mako_substitutions" ]] || {
  echo 'Pinned Mako/Posthorn 2x app, id, group, percent and escaped markup geometry differs'
  exit 1
}
./target/debug/examples/qa_png_probe \
  "$root/posthorn-substitutions-markup.png" '#E5C58B' 0 identical \
  "$root/mako-substitutions-markup.png"
echo 'Pinned Mako/Posthorn real 2x all ordinary notification format placeholders: MATCH'

# Hidden placeholder substitutions are not ordinary notification
# substitutions. Pinned notification.c:format_hidden_text separately
# resolves %h and %t; keep the visible summary constant while increasing
# the hidden count and compare entire decoded 2x compositor frames.
# The immediately preceding ordinary-placeholder comparison leaves its
# pinned Mako instance alive; release that private CI bus name before
# trying to launch this fixture's own fresh Mako process.
kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
daemon_pid=""
clear_color '#416D98'
cat >"$root/hidden-substitutions.conf" <<'EOF_HIDDEN_SUBS'
width=140
height=100
font=Sans 17
margin=0
padding=4
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%s
background-color=#416D98
text-color=#E5C58B
[hidden]
width=180
height=100
format=Hidden %h of %t
background-color=#9361B2
text-color=#E5C58B
EOF_HIDDEN_SUBS
hidden_notify(){
  local bus="$1" title="$2" response
  response="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    "$bus" /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Pinned Mako hidden format differential' 0 '' "$title" '' 0 0 30000)"
  [[ "$response" == u\ * ]] || {
    echo "hidden Notify failed: $response"; exit 1
  }
}
hidden_two_post=""
hidden_three_post=""
hidden_two_mako=""
hidden_three_mako=""
for impl in posthorn mako; do
  clear_color '#9361B2'
  if [[ "$impl" == posthorn ]]; then
    ./target/debug/posthorn --experimental-private-desktop "$root/hidden-substitutions.conf" \
      >"$root/daemon.log" 2>&1 &
    bus=org.posthorn.Development
  else
    "$mako_binary" -c "$root/hidden-substitutions.conf" >"$root/daemon.log" 2>&1 &
    bus=org.freedesktop.Notifications
  fi
  daemon_pid=$!
  wait_for_bus "$bus"
  hidden_notify "$bus" 'First hidden'
  hidden_notify "$bus" 'Constant visible'
  two_bounds="$(capture_bounds "$impl-hidden-two")"
  hidden_notify "$bus" 'Constant visible'
  # The purple popup retains exactly the same size when %h/%t change;
  # capture_bounds() can otherwise accept the OLD frame immediately.
  # Wait until the newly painted full screenshot actually differs from
  # the two-notification baseline, proving this compositor redraw.
  hidden_repainted=0
  for _ in {1..60}; do
    three_bounds="$(capture_bounds "$impl-hidden-three")"
    if ! ./target/debug/examples/qa_png_probe \
      "$root/$impl-hidden-three.png" '#9361B2' 0 identical \
      "$root/$impl-hidden-two.png" >/dev/null 2>&1; then
      hidden_repainted=1; break
    fi
    sleep 0.1
  done
  [[ "$hidden_repainted" == 1 ]] || {
    echo "Hidden %h/%t source fixture never repainted after third Notify: $impl"
    exit 1
  }
  echo "$impl scale-2 hidden substitutions after two: $two_bounds"
  echo "$impl scale-2 hidden substitutions after three: $three_bounds"
  if [[ "$impl" == posthorn ]]; then
    hidden_two_post="$two_bounds"; hidden_three_post="$three_bounds"
  else
    hidden_two_mako="$two_bounds"; hidden_three_mako="$three_bounds"
  fi
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
done
[[ "$hidden_two_post" == "$hidden_two_mako" &&
   "$hidden_three_post" == "$hidden_three_mako" ]] || {
  echo 'Pinned Mako/Posthorn real 2x hidden %h/%t popup bounds differ'
  exit 1
}
for count in two three; do
  ./target/debug/examples/qa_png_probe \
    "$root/posthorn-hidden-$count.png" '#9361B2' 0 identical \
    "$root/mako-hidden-$count.png"
done
if ./target/debug/examples/qa_png_probe \
  "$root/posthorn-hidden-two.png" '#9361B2' 0 identical \
  "$root/posthorn-hidden-three.png" >/dev/null 2>&1; then
  echo 'Changing the hidden count did not change the source-pinned placeholder raster'
  exit 1
fi
echo 'Pinned Mako/Posthorn exact scale-2 hidden %h/%t format and changed-count raster: MATCH'

# Pinned notification.c's %g is the ACTUAL mutable group_count,
# not always the singleton count. Create two messages in a
# group-by=app-name group while hiding the secondary group-index=1
# member. The visible leader retains the same summary, so the
# 1->2 group-count substitution alone changes its rendered glyphs.
cat >"$root/group-format.conf" <<'EOF_GROUP_FMT'
width=210
height=120
font=Sans 17
margin=0
padding=4
border-size=0
icons=0
max-visible=1
default-timeout=0
format=Group size: %g / %s
group-by=app-name
background-color=#416D98
text-color=#E5C58B
[group-index=1]
invisible=1
EOF_GROUP_FMT
group_format_notify(){
  local bus="$1" summary="$2" response
  response="$(busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    "$bus" /org/freedesktop/Notifications \
    org.freedesktop.Notifications Notify 'susssasa{sv}i' \
    'Pinned Mako mutable group format' 0 '' "$summary" '' 0 0 30000)"
  [[ "$response" == u\ * ]] || {
    echo "group format Notify failed: $response"
    exit 1
  }
}
for impl in posthorn mako; do
  clear_color '#416D98'
  if [[ "$impl" == posthorn ]]; then
    ./target/debug/posthorn --experimental-private-desktop "$root/group-format.conf" \
      >"$root/daemon.log" 2>&1 &
    bus=org.posthorn.Development
  else
    "$mako_binary" -c "$root/group-format.conf" >"$root/daemon.log" 2>&1 &
    bus=org.freedesktop.Notifications
  fi
  daemon_pid=$!
  wait_for_bus "$bus"
  group_format_notify "$bus" 'Stable visible group leader'
  first_group="$(markup_measure "$impl-group-one" "$bus")"
  group_format_notify "$bus" 'Secondary group member'
  changed_group=0
  for _ in {1..60}; do
    second_group="$(markup_measure "$impl-group-two" "$bus")"
    if ! ./target/debug/examples/qa_png_probe \
      "$root/$impl-group-two-markup.png" '#E5C58B' 0 identical \
      "$root/$impl-group-one-markup.png" >/dev/null 2>&1; then
      changed_group=1; break
    fi
    sleep 0.1
  done
  [[ "$changed_group" == 1 ]] || {
    echo "$impl group-count format did not visibly update after grouping"
    exit 1
  }
  echo "$impl source group count one: $first_group"
  echo "$impl source group count two: $second_group"
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
done
for count in one two; do
  ./target/debug/examples/qa_png_probe \
    "$root/posthorn-group-$count-markup.png" '#E5C58B' 0 identical \
    "$root/mako-group-$count-markup.png"
done
echo 'Pinned Mako/Posthorn exact 2x mutable %g group-count raster: MATCH'

# Startup CLI/style precedence through the actual PRIVATE Wayland desktop,
# not only --check-config or the headless D-Bus service. Both daemons load
# a width=144 config, then overwrite width to 205 and height to 103 from
# argv. The original pinned Mako parses -c and long args through getopt;
# Posthorn's explicitly private presenter must honor the same sequence.
cat >"$root/cli-desktop.conf" <<'EOF_CLI_DESKTOP'
width=144
height=165
font=Sans 17
margin=0
padding=4
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%s
markup=0
background-color=#416D98
text-color=#E5C58B
EOF_CLI_DESKTOP
cp "$root/cli-desktop.conf" "$root/cli-desktop-original.conf"
for impl in posthorn mako; do
  # Reset the file before each independent daemon run; preserve the
  # same CLI overrides across startup and Reload.
  cp "$root/cli-desktop-original.conf" "$root/cli-desktop.conf"
  clear_color '#416D98'
  if [[ "$impl" == posthorn ]]; then
    ./target/debug/posthorn --experimental-private-desktop "$root/cli-desktop.conf" \
      --width=205 --height=103 >"$root/daemon.log" 2>&1 &
    bus=org.posthorn.Development
  else
    "$mako_binary" -c "$root/cli-desktop.conf" --width 205 --height 103 \
      >"$root/daemon.log" 2>&1 &
    bus=org.freedesktop.Notifications
  fi
  daemon_pid=$!
  wait_for_bus "$bus"
  markup_notify "$bus"
  bounds="$(markup_measure "$impl-cli-desktop" "$bus")"
  echo "$impl physical-2x startup CLI style override: $bounds"
  # Config values changed on disk must reapply while the CLI width/height
  # override remains in force. This validates the real D-Bus Reload path
  # without claiming a production notification service.
  sed -i 's/^width=144$/width=300/; s/^height=165$/height=180/; s/^background-color=#416D98$/background-color=#2A7F9B/' \
    "$root/cli-desktop.conf"
  busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    "$bus" /fr/emersion/Mako fr.emersion.Mako Reload >/dev/null
  reload_ready=0
  stable_frames=0
  for _ in {1..60}; do
    if grim "$root/$impl-cli-reloaded.png" 2>/dev/null; then
      reload_bounds="$(./target/debug/examples/qa_png_probe \
        "$root/$impl-cli-reloaded.png" '#2A7F9B' 400 bounds 2>/dev/null || true)"
      if [[ -n "$reload_bounds" ]]; then
        # The first frame containing the NEW background can precede a
        # rebuilt layer surface with its final Pango bounds. Requiring
        # 3 consecutive identical fully decoded compositor frames
        # proves completion without hiding a stable geometry defect.
        if [[ -f "$root/$impl-cli-previous.png" ]] &&
            ./target/debug/examples/qa_png_probe \
              "$root/$impl-cli-reloaded.png" '#2A7F9B' 0 identical \
              "$root/$impl-cli-previous.png" >/dev/null 2>&1; then
          stable_frames=$((stable_frames+1))
        else
          stable_frames=0
        fi
        if (( stable_frames >= 3 )); then reload_ready=1; break; fi
        cp "$root/$impl-cli-reloaded.png" "$root/$impl-cli-previous.png"
      fi
    fi
    sleep 0.15
  done
  [[ "$reload_ready" == 1 ]] || {
    echo "$impl did not reach a stable post-Reload compositor raster"
    cat "$root/daemon.log"; exit 1
  }
  read -r cx0 cx1 cy0 cy1 <<< "$reload_bounds"
  (( cx1-cx0+1 == 410 )) || {
    echo "$impl lost --width=205 during disk Reload: $reload_bounds"
    exit 1
  }
  echo "$impl physical-2x post-Reload CLI width retention: $reload_bounds"
  # Pinned config.c:reload_config builds a replacement config separately;
  # when parsing fails, it returns before replacing the live configuration.
  # Neither desktop may lose its existing CLI override, redraw an invalid
  # style, or relinquish its private bus registration.
  sed -i 's/^width=300$/width=invalid/' "$root/cli-desktop.conf"
  if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --timeout=5s call \
    "$bus" /fr/emersion/Mako fr.emersion.Mako Reload \
    >"$root/$impl-invalid-reload.log" 2>&1; then
    echo "$impl incorrectly accepted malformed source config on Reload"
    exit 1
  fi
  grep -q 'Unable to parse configuration file' "$root/$impl-invalid-reload.log" || {
    echo "$impl did not return Mako's InvalidConfig Reload error"
    cat "$root/$impl-invalid-reload.log"; exit 1
  }
  sleep 0.25
  grim "$root/$impl-cli-rejected.png"
  ./target/debug/examples/qa_png_probe \
    "$root/$impl-cli-rejected.png" '#2A7F9B' 0 identical \
    "$root/$impl-cli-reloaded.png"
  echo "$impl rejected bad Reload without changing the settled desktop image"
  kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
  daemon_pid=""
done
./target/debug/examples/qa_png_probe \
  "$root/posthorn-cli-desktop-markup.png" '#E5C58B' 0 identical \
  "$root/mako-cli-desktop-markup.png"
./target/debug/examples/qa_png_probe \
  "$root/posthorn-cli-reloaded.png" '#2A7F9B' 0 identical \
  "$root/mako-cli-reloaded.png"
./target/debug/examples/qa_png_probe \
  "$root/posthorn-cli-rejected.png" '#2A7F9B' 0 identical \
  "$root/mako-cli-rejected.png"
echo 'Pinned Mako/Posthorn private Wayland CLI startup, valid Reload, and rejected Reload rasters: MATCH'

# T104: verify config.c's recursive include processing in the actual
# graphical daemon. The include target can be a symlink, children may
# include other children, and the parent may override the included
# root-style values AFTER the include. A second case verifies source
# expand_config_path("~//leaf") against a disposable HOME, without
# modifying a real user's environment or startup configuration.
mkdir -p "$root/include-home"
cat >"$root/include-leaf.conf" <<'EOF_INCLUDE_LEAF'
width=190
height=110
font=Sans 17
margin=0
padding=5
border-size=0
icons=0
max-visible=1
default-timeout=0
format=%s
markup=0
background-color=#416D98
text-color=#E5C58B
EOF_INCLUDE_LEAF
ln -s "$root/include-leaf.conf" "$root/include-alias.conf"
printf 'include=%s\nwidth=192\n' "$root/include-alias.conf" >"$root/include-middle.conf"
printf 'include=%s\nwidth=211\n' "$root/include-middle.conf" >"$root/include-root.conf"
cp "$root/include-leaf.conf" "$root/include-home/tilde-leaf.conf"
cat >"$root/include-tilde.conf" <<'EOF_INCLUDE_TILDE'
include=~//tilde-leaf.conf
width=207
EOF_INCLUDE_TILDE
for variant in nested tilde; do
  if [[ "$variant" == nested ]]; then
    config="$root/include-root.conf"
    expected_width=422
  else
    config="$root/include-tilde.conf"
    expected_width=414
  fi
  for impl in posthorn mako; do
    clear_color '#416D98'
    if [[ "$impl" == posthorn ]]; then
      HOME="$root/include-home" ./target/debug/posthorn \
        --experimental-private-desktop "$config" >"$root/daemon.log" 2>&1 &
      bus=org.posthorn.Development
    else
      HOME="$root/include-home" "$mako_binary" -c "$config" \
        >"$root/daemon.log" 2>&1 &
      bus=org.freedesktop.Notifications
    fi
    daemon_pid=$!
    wait_for_bus "$bus"
    markup_notify "$bus"
    result="$(markup_measure "$impl-include-$variant" "$bus")"
    echo "$impl source-pinned $variant include geometry and text: $result"
    read -r ix0 ix1 iy0 iy1 itext <<<"$result"
    (( ix1-ix0+1 == expected_width && itext > 250 )) || {
      echo "$impl did not apply the included style and final parent width"
      exit 1
    }
    kill "$daemon_pid"; wait "$daemon_pid" 2>/dev/null || true
    daemon_pid=""
  done
  ./target/debug/examples/qa_png_probe \
    "$root/posthorn-include-$variant-markup.png" '#E5C58B' 0 identical \
    "$root/mako-include-$variant-markup.png"
  echo "Pinned Mako/Posthorn real scale-2 $variant included config raster: MATCH"
done
