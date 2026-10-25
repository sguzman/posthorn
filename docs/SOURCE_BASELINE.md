# Mako source baseline and port policy

## Pinned upstream

- Upstream: https://github.com/emersion/mako
- Baseline: **v1.11.0**, published 2026-03-26.
- **Version boundary:** post-v1.11.0 Mako `master` commit `cce37cd3405f64eee1b07d4656b5ecbbc912b46d` (2026-06-30) changed the trailing bottom-margin behavior. The pinned **v1.11.0** `render.c` does **not** add `pending_bottom_margin` to the final `rendered_height`; Posthorn deliberately retains that omission. Later upstream behavior must not silently enter the strict-port MVP. Full runtime parity remains unverified.
- Mako license: MIT; credit preserved here. Posthorn code is an independent Rust implementation, not a copy-paste translation of upstream C.

## Strict MVP contract

Posthorn must replace a modern Mako daemon without asking applications to
change how they call the Freedesktop Notification protocol. Until parity:

- Preserve Mako INI-like configuration semantics; **do not move to TOML**.
- Preserve Mako category criteria, mode overrides, colors, Pango formatting,
  sound hooks, timeouts, actions, icons, grouping, sorting and history.
- Match CLI control operations and signals/properties, including the Mako 1.11
  JSON history/list support and Notifications changed property.
- Support layer-shell popups on Hyprland, including correct monitor, anchors,
  margins, scaling, placement, focus/activation, pointer and touch behavior.
- Retain in-memory history and all associated Mako semantics. SQLite, a GUI,
  general channels, and a broker are **explicitly deferred**.
- Keep Mako active in `config-store` until side-by-side tests prove parity.
- Never try to run both notification servers on the same session bus name.

## Signed padding in pinned Mako v1.11.0

Mako `types.c:parse_directional` accepts signed integers for padding.
The pinned `render.c:render_notification` retains these signed top,
bottom, left, and right padding values directly in Pango text bounds,
text offsets, icon positioning, and computed notification height.
Posthorn previously clamped negative directional padding to zero when
measuring and painting, changing geometry for a supported config.

The renderer now preserves signed insets while bounding physical SHM
allocations and Pango layout dimensions. A source-matched regression
demonstrates that a -3px top and -2px bottom inset reduce notification
height by 5px and that Cairo uses that same measured height. Full Rust,
private D-Bus, and isolated 1×/1.25×/2×/mixed-DPI Sway CI passed for
[9196333](https://github.com/sguzman/posthorn/actions/runs/40055215486).
A second Rust regression checks Mako's signed *horizontal*
insets: negative left/right padding expands the Pango wrapping region
and reduces the wrapped notification height. The full suite at
[1d43f2d](https://github.com/sguzman/posthorn/actions/runs/40055659262)
is green.

An independent private Sway screenshot acceptance also renders a live
notification and switches its mode among zero padding, -3px top/-2px
bottom, and back to zero. The measured popup heights must shrink
exactly 5px and then recover without restarting or replacing the
notification. Required headless CI passed at
[9149e52](https://github.com/sguzman/posthorn/actions/runs/40055526248).
This validates signed insets and real Sway redraw, not exact complete
Pango pixel parity on the user's physical Hyprland session.

## Scale-aware notification hitboxes and themed icons

Pinned Mako v1.11.0 `icon.c` selects themed assets using the **maximum
integer scale among all attached outputs**, rather than only the target
popup monitor. Distinct exact-match 1× and 2× theme assets can have
different aspect ratios. Before this correction Posthorn rendered such
an icon at the correct scale but measured the pointer hitbox and
`wl_surface` input region using the 1× asset, potentially routing clicks
through a visible notification or intercepting clicks in an empty gap.

The input geometry, single/multi-surface click resolver and atomic
input-region commits now reuse the **same icon scale** as the renderer.
A fixture provides differently shaped actual 1× and 2× PNG assets and
asserts that logical notification rectangles equal the measured raster
height, including a 2× physical SHM buffer. Both `cargo test --all-targets`
and private D-Bus/headless Sway acceptance passed at
[94c7534](https://github.com/sguzman/posthorn/actions/runs/40054229374).
This is implementation and private-compositor evidence, not physical
mixed-DPI Hyprland monitor acceptance. The original installed Mako
service and production notification name remain unchanged.

## Real compositor mixed-DPI icon selection and pointer extent

The private two-output Sway acceptance fixture creates a 1× notification
on HEADLESS-1 while toggling HEADLESS-2 between 2× and 1×. It verifies
the selected 2× and 1× icon colors and the resulting actual popup heights,
then sends Wayland virtual-pointer clicks near the painted bottom edge to
prove that the live input region follows reflow. The test makes no requests
on the workstation D-Bus or Wayland session. The complete Rust, private
D-Bus and 1×/1.25×/2× Sway CI matrix passed at
[commit c06ecfd](https://github.com/sguzman/posthorn/actions/runs/40054674663).
This is not acceptance on physical Hyprland monitors or touch devices.
Production Mako remains untouched.

## Pinned v1.11.0 final notification margin

Upstream `render.c` adds the first top margin and the greater of adjacent
top/bottom margins when stacking notifications. Its final `rendered_height`
omits the last pending bottom margin. Posthorn previously implemented a
later Mako `master` correction, which incorrectly grew the bottom of its
strict-v1.11 surface. The source-backed correction is in
[`d1ab406`](https://github.com/sguzman/posthorn/commit/d1ab40618ebdac22c1693a4e3b1f5116f4e32d76).
The corresponding per-category desktop fixture was corrected at
[`a5a638e`](https://github.com/sguzman/posthorn/commit/a5a638e416d9b51fc987fe6e37a5a7d29bab112a).
Only passing CI for the latter counts as acceptance.

## Pointer cursor and compositor input regions

Pinned Mako v1.11.0 `wayland.c` selects the compositor's default
`wp_cursor_shape_manager_v1` cursor at `wl_pointer.enter`, falling back to
the theme's `left_ptr` cursor when that protocol is absent. Posthorn's
private presenter now follows both branches using the Rust Wayland protocol
and cursor crates. The combined implementation passed Rust, private D-Bus,
and all isolated compositor jobs at
[953bb0a](https://github.com/sguzman/posthorn/actions/runs/40046958828).

Mako also sends a per-surface `wl_region` union of actual notification
hotspots, rather than making the entire popup layer-shell canvas clickable.
Posthorn's private single- and multi-surface renderers now apply input
rectangles from the **same geometry computation used by binding hit-tests**;
transparent margins and inter-notification gaps are left outside that
registered input region. The new unit regression and the complete isolated
Sway suite passed at
[af4b268](https://github.com/sguzman/posthorn/actions/runs/40047135969).

An additional overlapping two-layer compositor probe sends two real virtual
pointer clicks: one through the overlay's excluded input region, received
by the underlying Top layer, and one inside the overlay region, received
only by the Overlay layer. Both Rust and isolated Sway jobs passed at
[3158fde](https://github.com/sguzman/posthorn/actions/runs/40047511507).
This verifies **actual Sway input pass-through**, not just a geometry unit
test or screenshot. Physical Hyprland delivery, real touchscreen behavior,
compositor cursor-theme appearance, and approval to replace installed Mako
remain separate hard gates. The production Mako service, D-Bus name,
autostart, and workstation configuration remain untouched.

Posthorn's private desktop now submits each frame and its exact notification
`wl_region` in **one Wayland commit**, both on initial surface mapping and
same-size buffer redraw. This prevents an intermediate committed frame from
temporarily claiming input over transparent margins. The guarded private Sway
probe remaps the top overlay input region in-place, then verifies four real
pointer deliveries to the correct upper/lower layer. Full Rust and Sway CI
passed at [33f80ed](https://github.com/sguzman/posthorn/actions/runs/40053422778).
A separate unit regression verifies adjacent notification hotspots and that
their max(bottom,top) transparent gap is not click-targetable; full CI passed
at [2363c25](https://github.com/sguzman/posthorn/actions/runs/40053551698).
Neither private compositor suite substitutes for real workstation Hyprland QA.

## Port map

| Upstream source | Posthorn scope | State |
| --- | --- | --- |
| `config.c`, `criteria.c`, `types.c` | Config parser, criteria, styles, types | Partial; stricter Mako selector validation (bare booleans only, anchor names, last repeated field wins, byte-oriented line/value whitespace and Mako LP64 strtol parsing (including empty values and signed 32-bit narrowing) shared by validation, rendering, hit-testing, limits and timeout scheduling), legacy/XDG default configuration lookup, signed max-history disabling, format/group validation, POSIX ERE newline and glibc escaped-ASCII-letter/bracket behavior, plus rejection of Rust-only inline group extensions, Mako progress-color tokenization and blend operators, and transactional reload |
| `notification.c` | IDs, replace, close, timeout, history, groups, format | Timers, dismiss/restore with persistent restored notifications and live mode/reload history-policy reapplication; insertion-time group-by passes with persisted indices/counts, physical ListNotifications grouping order under ascending/descending time sort, regrouping on close/expiration and mode/reload, Mako's bounded [-1,2] priority insertion search (including invalid raw urgencies) with private D-Bus acceptance, indexed grouped visibility (including scoped category group-by and explicit secondary-member overrides), per-output/anchor limits and overflow placeholders tested; replacement-time regrouping and exact surface association remain partial |
| `dbus/xdg.c`, `dbus/mako.c`, `dbus/dbus.c` | Desktop interface, Mako control interface | Private development bus with image-data, image-path, progress, reload, Chromium uint32/int32 urgency, missing-hint unknown urgency (wire byte 255), and strict known-hint type validation, strict typed DismissNotifications variant rejection and arrival-ordered Notify aliases (including empty image-path and duplicate hints), signed raw urgency y/u/i including out-of-range values, control urgency bytes, and urgency-criteria matching verified on an isolated D-Bus session, Mako superstyle-based GetCapabilities, including built-in grouped body and scoped criteria, ActivationToken ordering, Mako-style actions=0 signal suppression and dynamic mode re-enablement, and on-notify action/dismiss signal tests; full protocol parity pending |
| `wayland.c`, `surface.c`, `pool-buffer.c` | Wayland, layer-shell, compositor surfaces | Private desktop has stacked SHM layer-shell surfaces with redraw reuse and release-aware SHM buffers, dynamic named-output registration/removal, native-resolution integer-scaled Cairo/Pango popups, XDG activation token requests, pointer/touch, anchors, and outer-margin placement; live compositor QA pending |
| `render.c`, `icon.c`, `cairo-pixbuf.c` | Pango/Cairo rendering, layout and icons | Cairo/Pango markup/colors/file and inline icons with Mako-style HiDPI theme selection using the maximum scale across attached outputs, independently of the popup monitor scale (verified with synthetic 1x/2x PNG assets), progress fill, measured heights, group counts, directional text/icon padding, per-corner border radius, icon clipping, signed-margin overlap and ARGB composition; fractional-scale-specific presentation and exact Mako Pango metrics remain incomplete |
| `event-loop.c`, `main.c`, `mode.c`, `makoctl.c` | Runtime, modes, timers, control CLI | Private live/headless services, post-grouping on-notify hooks and quiet-mode sound tests, Mako-style `mode -s`, GNU getopt compact arguments, printed mode results, duplicate suppression and mutually exclusive -a/-r/-t validation, text history/list and upstream-style JSON formatting, transactional reload, dynamic group/list reapplication on SetMode/Reload, Mako-style suppression of action and activation signals when actions=0; XDG token protocol covered headlessly, GNU getopt lone-dash handling, strtoumax uint64 negative-wrap ID parsing and C-whitespace handling, getline-compatible menu selection; exact control parity incomplete |

## Ordered actions and map-backed fallback

Mako 1.11 writes action entries in internal linked-list order, including repeated
keys, into its `a{ss}` control D-Bus field. Posthorn now preserves those
entries in `ListNotifications` and `ListHistory` through a custom ordered
serializer. Its `posthornctl` client uses a matching ordered deserializer for
menu selection, human listing, and JSON listing. The private D-Bus roundtrip,
binary encoding, and menu selection have passed
[CI acceptance](https://github.com/sguzman/posthorn/actions/runs/39866232367).
The standard control object's `Properties.Get` and `GetAll` now use a
path-local ordered serializer instead of the zbus-generated OwnedValue
conversion. Both retain repeated `a{ss}` keys in Mako's linked-list
order. The private D-Bus regression compares actual wire bytes for
Get, GetAll, ListNotifications and ListHistory and confirms standard
property signatures, read-only errors, and PropertiesChanged
introspection. All jobs passed [CI at 8ecf716](https://github.com/sguzman/posthorn/actions/runs/39952699403).
The old map-backed accessor remains an internal compatibility helper and
is not the external property wire path. D-Bus dictionaries conventionally
require unique keys, although Mako itself emits duplicates. Full daemon
runtime and compositor parity remain unproven.

## Isolated D-Bus name ownership and collision behavior

On the private `dbus-run-session` test bus, Posthorn requests
`org.freedesktop.Notifications` with `DoNotQueue`. A second attempted
standard-name server must fail immediately, and a subsequent
`org.freedesktop.DBus.GetNameOwner` query must still report the original
owner. The same assertions exercise two instances of
`org.posthorn.Development` to ensure the development entrypoint does not
have different queue or replacement behavior. This test **never**
contacts the workstation's real session bus or installed Mako.

The Rust/private D-Bus job and all headless Sway acceptance scales passed at
[commit f121917](https://github.com/sguzman/posthorn/actions/runs/40014316439).
This verifies bus-name collision handling but is not permission for the
production cutover; real Hyprland and whole-daemon feature parity remain
hard prerequisites.

## Sender-ordered dismissal options

Mako 1.11 parses `DismissNotifications`' `a{sv}` arguments in wire order.
An earlier malformed known option must cause rejection even when a later
duplicate supplies a valid value; last valid occurrences otherwise win.
Posthorn now deserializes arrival-ordered entries and validates each before
constructing the internal options map. Both the parser unit test and the
[private D-Bus duplicate-key regression](https://github.com/sguzman/posthorn/actions/runs/39896396119)
passed Rust and headless Wayland CI at commit `e362363`. This fixes
dismissal input parity independently of the now-verified ordered property path.

## POSIX ERE criteria compatibility

Mako 1.11 calls glibc's regcomp with REG_EXTENDED | REG_NOSUB
(no REG_NEWLINE). Posthorn now preserves dot-matches-newline without
multiline anchors, rejects Rust-only inline groups such as (?i), and
normalizes glibc's escaped ASCII-letter semantics before Rust compilation:
outside bracket classes only \b, \B, \w, \W, \s and \S retain their
GNU special meanings; within classes all escaped ASCII letters are literal.
Nested POSIX character-class delimiters are handled independently.
Tests cover format selectors, class membership, and actual private D-Bus
on-notify matching. The latest regression passed
[full CI](https://github.com/sguzman/posthorn/actions/runs/39946521970).
This is **not** full POSIX ERE parity: glibc accepts capture backreferences
such as (ab)\1 that the Rust regex crate does not. Locale-sensitive
character classes and other grammar differences still need acceptance tests.

## Mako-style padding and corner-radius resolution

Mako's config.c clamps left/right padding to at least the corresponding
border-radius when both are specified in the same style. It preserves the
clamped result across later radius reassignments. The root style already
specifies padding and radius in its defaults, but a user criteria style begins
empty. Overriding only one field in a later scoped rule does not re-clamp the
inherited field.

Posthorn now records configuration-time padding/radius assignment state
separately from the unchanged raw config value and applies its effective
padding during style resolution. Root, scoped, repeated, and later-override
regressions passed [full CI](https://github.com/sguzman/posthorn/actions/runs/39947427676)
at commit d621893. Direct render measurement regression is separately
tracked. Further layout and compositor parity remains an MVP gate.

## Source-matched color parsing and deep includes

Mako v1.11 `types.c:parse_color` requires a `#` prefix and exactly
six or eight following bytes, but parses those bytes with C
`strtoul(..., base=16)`, then truncates to uint32 and appends opaque
alpha for six-byte inputs. Posthorn now reproduces the allowed C-locale
leading whitespace, unary signs, 0x prefixes, and signed wraparound
instead of requiring each byte to be a hex digit. Tests compare known
C fixture values, config acceptance, and exact Cairo background pixels.

Mako's `config.c:apply_config_option` recursively loads included files
without a fixed nesting limit. Posthorn no longer rejects the 17th
nested include: 22-file chains are exercised through `Config::from_path`
and real private D-Bus Reload. Malformed leaf configurations and
include cycles reject the reload transactionally, retaining the last
good configuration. Canonical include-cycle detection is a safety
divergence from Mako's unbounded recursion, not proof of complete
configuration parity.

The regression suite passed both CI jobs at
[commit 3f3754a](https://github.com/sguzman/posthorn/actions/runs/39950069729).
These findings do not close remaining POSIX grammar, properties, or
real Hyprland parity gates.

The nested-include precedence regression also verifies that recursively
included criteria retain source order, that the parent file can
override included root options after the `include=` directive,
and that later matching criteria in the parent override their
earlier included counterparts. The test uses three independently
stored synthetic files and checks both effective style and source
locations. All Rust/private D-Bus and isolated Sway 1×, 1.25× and
2× jobs passed at
[commit e3c2f00](https://github.com/sguzman/posthorn/actions/runs/40027375107).
This strengthens configuration-source parity but does not claim
complete glibc regex or user-workstation config acceptance.

## Fail-fast D-Bus name acquisition

Upstream Mako v1.11 registers its interfaces in `dbus/dbus.c` before
requesting `org.freedesktop.Notifications` with `sd_bus_request_name(..., 0)`.
Posthorn now builds both notification and control objects, installs its
ordered `Properties.Get`/`GetAll` serializer, and only then requests
its selected well-known name with `RequestNameFlags::DoNotQueue`.
Default queued-name behavior is prohibited: an already-owned bus name
returns `NameTaken` immediately and does not replace the current owner.

On an isolated `dbus-run-session`, integration tests exercise simultaneous
standard-name requests from two Posthorn instances and collisions against
an unrelated owner of `org.posthorn.Development`. They verify that the
original owner remains unchanged and that the second service fails instead
of joining the name queue. Both Rust/private D-Bus and headless Wayland CI
passed at [commit 614055e](https://github.com/sguzman/posthorn/actions/runs/39955644267).
No workstation session bus, Mako service, or user configuration was touched.
These startup safeguards do **not** authorize production name acquisition;
that still requires the explicit reversible MVP inauguration gate.

## Makoctl CLI option and menu parity

Mako v1.11 `makoctl.c` parses notification IDs using C
`strtoumax(...,10)`. Posthorn now accepts its leading whitespace,
single optional sign, uint64 negative wrap and empty-input cases,
while rejecting repeated signs such as `++1` and `-+0`.
Regression tests cover direct IDs and all three `-n` command forms:
`dismiss`, `invoke` and `menu`.

Mako's GNU `getopt("a:r:t:s")` consumes mixed short-option clusters
one option at a time. Posthorn now handles consecutive `-s` switches
followed by argument-taking `-a`/`-r`/`-t` flags, including attached
arguments and the existing mutual-exclusion rule.

Upstream `makoctl menu` reads only the first line from the menu
program's stdout via `getline`, closes stdout before `waitpid`,
and ignores failed label writes to an already-closed child stdin.
Posthorn now does the same with buffered first-line byte reads, not
whole-output UTF-8 conversion. Both invalid bytes after a valid
selection and early child stdin closure are covered by real subprocess
tests; an isolated private D-Bus acceptance test verifies that the
selected action emits `ActionInvoked`.

These cases passed Rust, D-Bus integration and headless Wayland CI
at [commit 3c04c97](https://github.com/sguzman/posthorn/actions/runs/39959346678).
Exact CLI output and error wording still have outstanding parity cases.

## Category sound-hook routing and quiet-mode verification

Mako v1.11 implements notification sounds via configured
`on-notify=exec` bindings rather than a built-in audio engine.
The isolated private D-Bus regression now sends success, failure and
warning notifications, verifies each resolves its own configured
background color and subprocess hook, and records exactly one command
per category. After switching to `quiet` through the Mako control
interface, all three command hooks are suppressed while their
notification objects and category colors are retained.

The test uses file-backed shell commands rather than sound hardware.
It proves category routing and quiet-mode suppression, **not** speaker,
PipeWire or live Hyprland audio acceptance. Both Rust/D-Bus and
headless Wayland CI passed at
[commit b57ff88](https://github.com/sguzman/posthorn/actions/runs/39960413355).
Actual sound-player integration and workstation cutover remain open.

## Configured exec-hook standard streams

Mako v1.11 `notification.c:notification_execute_binding` double-forks and
executes `sh -c` without redirecting its inherited stdin, stdout, or
stderr. Posthorn previously redirected all three descriptors to
`/dev/null`, suppressing command output and diagnostics. Configured
`on-notify=exec` and mouse/touch `exec` bindings now explicitly inherit
the parent's standard streams. Notification-originated fields are still
not interpolated into the shell, apart from Mako's decimal `$id`.

Regression tests launch the hook in a separate test-harness process with
real OS pipes, checking that stdout, stderr and stdin reach the executed
shell. The tests and existing private D-Bus/headless Wayland suites passed
[CI at 18cbe4e](https://github.com/sguzman/posthorn/actions/runs/39965029992).
This is standard-stream fidelity, **not** real audio-player or Hyprland
speaker acceptance.

## Icon theme scale selection from Mako's source path

Mako v1.11 `icon.c:resolve_icon` searches `root/*/*/name.*`. It
parses the nominal size from the first wildcard path component and
falls back to the second only when the first parses as zero or overflows.
After that selection, the pinned C source calls
`strchr(relative_path, '@')`: the search begins at the size-bearing
component but **continues through the remaining category directory
and filename**, taking the first `@` it finds. Thus
`64x64/apps@2/name.png` has scale 2, whereas
`apps@2/64x64/name.png` has scale 1 because the parser has already
advanced past `apps@2`. The earlier claim that only the size-bearing
component supplies the scale was incorrect.

Posthorn now matches this remaining-path scan, retaining the existing
signed C `strtol` semantics and nominal-size narrowing. Source-matched
regressions test both directory layouts, a filename `@2` suffix,
first-`@` precedence (including an invalid suffix), invalid first
size fallback, actual differently colored PNGs at 1× and 2×, and
unchanged popup geometry. Both Rust/private D-Bus and isolated Sway
scaling, multi-output and pointer acceptance passed at
[commit d2e4e00](https://github.com/sguzman/posthorn/actions/runs/40122881673).
This does not prove live mixed-DPI Hyprland monitor acceptance, and
the 99% T402 parity gate remains incomplete.

A second isolated acceptance test, `tests/private_theme_scale.rs`, sends
a Freedesktop `Notify` carrying the themed app-icon name through a private
D-Bus service, then rasterizes the **received notification** against two
synthetic PNGs in `16x16/apps` and `16x16/apps@2`. It verifies an actual
red pixel at 1× and green pixel at 2× without changing the logical
popup geometry. This exercises D-Bus ingestion, configured icon-path,
GdkPixbuf decoding, theme search, and Cairo painting together.
Both CI jobs passed at
[commit 8be2bfd](https://github.com/sguzman/posthorn/actions/runs/40123319975).
Physical Hyprland and production Mako cutover remain untested and
unauthorized.

## Relative and non-UTF-8 file icon URLs

Mako v1.11 `icon.c:resolve_icon` strips the `file://` prefix and
percent-decodes the path verbatim. It does not require an absolute path.
Posthorn previously rejected relative `file://` paths and any decoded
filename containing non-UTF-8 Linux bytes. The Linux-native decoder now
retains raw filename bytes in an `OsString` and supports relative paths,
without relaxing confined theme-name validation or allowing embedded NUL.

Tests verify decoded relative, absolute, malformed-percent, and non-UTF-8
filenames. Cairo tests construct genuine PNG fixtures, including a
non-UTF-8 filename, and confirm the expected icon pixels. Both Rust,
isolated D-Bus and headless Wayland CI passed at
[commit 7b72418](https://github.com/sguzman/posthorn/actions/runs/39980888535).
The embedded-NUL rejection remains an intentional safety divergence
from C's truncating string representation. Real Hyprland acceptance
and complete theme fidelity remain open.

## Raw-byte icon hints and integer progress fill

Mako 1.11 resolves sender-provided `image-path` hints through its
regular icon loader. In addition to direct Cairo fixture tests,
Posthorn now sends a percent-encoded non-UTF-8 Linux filename through
the real private `org.freedesktop.Notifications.Notify` method and
verifies that the selected icon bytes are painted into the raster
([commit 1cbcf08](https://github.com/sguzman/posthorn/actions/runs/39983942994)).

Mako `render.c:render_notification` uses **integer division** for
the progress fill width: `(width - 2 * border_size) * progress / 100`,
before Cairo receives the rectangle. Posthorn previously drew a
fractional-width rectangle, which could blend its trailing pixel.
The renderer now uses Mako's integer logical-width calculation before
HiDPI scaling. Regression tests inspect solid filled/unfilled
pixel boundaries for several percentages at 1× and 2×. A private
D-Bus integration test also sends `value=33` and validates the
exact painted boundary through the sender-to-renderer path.
Both CI jobs passed at
[commit c704039](https://github.com/sguzman/posthorn/actions/runs/39984630050).

These are isolated renderer and protocol tests, **not** proof of
complete live Hyprland presentation, interaction or cutover parity.

## Overlapping mouse hotspots and 24.8-bit Wayland input coordinates

Pinned Mako v1.11 `wayland.c:pointer_handle_button` and
`wayland.c:touch_handle_up` iterate the active notification list,
selecting the **first full rectangular hotspot match**. Posthorn
previously stopped on a Y-axis match even when X missed a narrow
topmost notification. It now continues to the next hotspot, allowing
a wider overlapping notification underneath to receive the click.
A unit test verifies both the fallback target and first-match
precedence; a private D-Bus test sends two differently sized
overlapping notifications, dispatches the click, and checks that
only the intended underlying notification enters history.

Mako converts Wayland's 24.8 fixed-point coordinates via
`wl_fixed_to_int` on pointer enter/motion and touch down/motion.
The underlying `wayland-util.h` operation is `f / 256`, i.e.
integer truncation toward zero even for negative fractions.
Posthorn now quantizes incoming Wayland coordinates identically,
with positive and negative boundary tests. Full Rust/private D-Bus
and headless Wayland CI passed at
[commit 701e3a0](https://github.com/sguzman/posthorn/actions/runs/39988108025).

These tests do **not** replace compositor-originated Hyprland
pointer, touch, or XDG activation acceptance; those remain open.

## XDG activation replies can outlive a Wayland sync roundtrip

Pinned Mako v1.11 `wayland.c:create_xdg_activation_token` calls
`xdg_activation_token_v1.commit`, then dispatches Wayland events
until the compositor returns `xdg_activation_token_v1.done`.
Posthorn formerly performed only one `wl_display.sync` roundtrip,
which does not guarantee that the asynchronously generated
activation token has arrived.

Posthorn now polls and dispatches events for the **matching**
token object until its Done event or a 500 ms deadline. A late reply
to an earlier request cannot be reused for another click.
The timeout is a deliberate safety limit, unlike Mako's unbounded
waiting loop. The code compiles and the existing Rust/private
D-Bus plus headless Wayland regression suites pass at
[commit 1c91a75](https://github.com/sguzman/posthorn/actions/runs/39992161443).
Those suites do not emulate a compositor that deliberately delays
activation Done, and they do not prove real Hyprland activation.
Live XDG activation acceptance remains open.

## Real Wayland pointer delivery in isolated headless Sway

The headless compositor initially advertises `wl_seat.capabilities=0`:
a `swaymsg seat cursor press` command cannot reach Posthorn because no
pointer device exists. Posthorn's development-only test harness now
uses a **Rust wlroots virtual-pointer client** to register a temporary
pointer on the isolated Sway seat, wait for `wl_seat.pointer`
capability, send a real right-click and release, then destroy the
device. This exercises Mako-style hotspot processing through actual
Wayland protocol events, not just the in-memory hit-test.

The 1× acceptance test shows three different category colors,
sends the input event to the newest warning popup, and verifies
through a fresh compositor screenshot that only that popup disappears
while the older two remain. The existing 1.25×/2× repaint, timer,
reload, mode and named-output hotplug tests still pass.
`examples/qa_virtual_pointer.rs` explicitly refuses to run without
`POSTHORN_HEADLESS_POINTER_TEST=1` and
`WLR_BACKENDS=headless`; the test launcher sets them only in its
private runtime directory. These tests passed
[CI at commit 4003cf2](https://github.com/sguzman/posthorn/actions/runs/39994717557).

This is **headless Sway** mouse acceptance, not real Hyprland
pointer/touch or compositor-issued XDG activation acceptance.
No production D-Bus name was taken and no installed Mako configuration
was changed.

## Scaled compositor pointer and activation-token acceptance

The guarded compositor test now emits real right-button input against
independent isolated Sway outputs at 1× and 1.25×, with the fractional
output's pointer coordinates expressed in **logical** dimensions
(1536×864 from the 1920×1080, 1.25× configured mode), not in screenshot
pixels. Both Rust/private D-Bus and compositor CI passed at
[commit 68570a7](https://github.com/sguzman/posthorn/actions/runs/40000871778).
The independent 2× smoke retains its Freedesktop CloseNotification
regression and additionally injects a right-click with a 960×540
logical output. That 2× compositor job passed at
[commit 19d749c](https://github.com/sguzman/posthorn/actions/runs/40001253136);
the separate Rust job was pending at the time of this documentation.

A real compositor-originated left-button click on a notification with
a sender-provided `default` action now exercises the full activation
path. `WAYLAND_DEBUG=client` shows `xdg_activation_v1.get_activation_token`,
`set_serial`, `set_surface`, `commit`, and an asynchronous
`xdg_activation_token_v1.done` containing a nonempty compositor-issued
token. The isolated D-Bus monitor checks that **the exact same token**
appears in Freedesktop `ActivationToken` *before* `ActionInvoked`
and `NotificationClosed`; screenshots check that the action popup
disappears while older notifications remain. This acceptance passed the
1× headless compositor job at [commit 19d749c](https://github.com/sguzman/posthorn/actions/runs/40001253136).
Clicks outside a popup and unbound middle clicks are also checked for
non-dismissal and absence of an unnecessary activation request.

Mako v1.11 calls `create_xdg_activation_token` for a matching
sender action **before** checking whether the resolved style permits
action signals. The 1× compositor test now reloads a scoped
`actions=0` rule, sends a notification with a real default action,
and clicks it with the temporary virtual pointer. The test verifies
that the token request still occurs, the target popup disappears,
and neither `ActivationToken` nor `ActionInvoked` is emitted.
The complete headless Wayland job passed at
[commit b27949e](https://github.com/sguzman/posthorn/actions/runs/40002064939);
the separate Rust CI job remained in progress when this was recorded.

These are authentic **headless Sway** protocol observations, not mocked
tokens. They still do **not** prove production Hyprland action focus,
compositor touch behavior, exact fractional-buffer negotiation, physical
audio, or the reversible Mako replacement gate.

## Real compositor action and custom button edge cases

Two additional isolated Sway 1× input regressions use the guarded
wlroots virtual pointer instead of synthetic hit tests. When a sender
supplies an `open` action but **no `default` key**, Mako's default
left-button binding still dismisses its popup without an
`xdg_activation_v1.get_activation_token` request or
`ActivationToken`/`ActionInvoked` signals. This passed the full
Rust/private D-Bus and 1×/1.25×/2× compositor CI at
[commit c109ae8](https://github.com/sguzman/posthorn/actions/runs/40009309154).

A notification scoped by `[category=middle-dismiss]` overrides the
previously unbound middle mouse button with
`on-button-middle=dismiss --no-history`. Actual
`wl_pointer.button` delivery must remove the target popup while
leaving older unrelated popups intact, emit
`NotificationClosed` without invoking an action or requesting XDG
activation, and omit the dismissed notification from the control
`ListHistory` result. Both CI jobs passed at
[commit 05a1353](https://github.com/sguzman/posthorn/actions/runs/40009694365).

These prove additional *headless Sway* input and D-Bus behavior only;
no physical Hyprland pointer/touch or production session ownership is
implied.

## Grouped pointer dismissal and click-instance isolation

A scoped `group-by=app-name` and
`on-button-right=dismiss-group` fixture creates two live notifications
from one sender. A compositor-originated right click on the visible
group leader must emit two `NotificationClosed` signals, remove both
members from `ListNotifications`, and preserve unrelated popups.
It must not request an XDG activation token. The full private Rust/D-Bus
and isolated Sway suite passed at
[commit e6ce960](https://github.com/sguzman/posthorn/actions/runs/40010258132).

There was also a dispatch race: the private Wayland thread originally
hit-tested a pointer event to decide whether to request XDG activation,
waited asynchronously for the compositor's reply, then sent bare
coordinates to the D-Bus thread, which hit-tested *again*. A new popup
appearing during the token wait could steal the original click.
Posthorn now captures the originally hit-tested notification ID,
binding and internal active incarnation. Before dispatch, it verifies
that this **same incarnation** is still active and unchanged, never
retargeting a stale click. Every Notify replacement (explicit ID or
app-scoped stack tag), including byte-identical replacements, gets a
fresh internal incarnation; restoring history also starts a new active
incarnation while preserving the external Freedesktop ID. The input
handoff avoids cloning every notification and inline image by snapshotting
small (ID, incarnation) pairs first.

Unit acceptance checks concurrent-new-popup stability, removed targets,
changed and byte-identical replacements, stack-tag replacement and
history restoration. The full Rust/private D-Bus and isolated Sway suite
passed at [commit 915a07a](https://github.com/sguzman/posthorn/actions/runs/40011789720).
No real Hyprland touch or focus handoff is claimed.

## Notification action and group dismissal without D-Bus await races

Pinned Mako v1.11 processes group dismissal and action invocation in one
synchronous event-loop turn. Posthorn previously awaited
`NotificationClosed` between each member in `dismiss-group` and
`dismiss-all`, and awaited `ActivationToken`/`ActionInvoked` *before*
closing the clicked notification. A sender replacing an ID during those
awaits could have a newly arriving notification dismissed by the old
click.

The input adapter now closes all selected group/all members before
awaiting their respective D-Bus signals. Similarly it resolves a
sender's default or named action and removes the original notification
**before** the first asynchronous action signal. On-wire ordering remains
`ActivationToken` (when applicable), `ActionInvoked` (when applicable),
`NotificationClosed`, followed by the control property's invalidation.
`actions=0` and missing default actions still dismiss without action
signals, and a missing ID emits nothing. The tests check group
membership/history and reject duplicate closures; the private
compositor suite checks action signal order and grouped dismissal.

The grouped/all implementation passed both CI jobs at
[commit 3bec532](https://github.com/sguzman/posthorn/actions/runs/40019970174).
The action implementation and fixed regression fixture passed both CI
jobs at [commit 8299ed5](https://github.com/sguzman/posthorn/actions/runs/40020352282).
This proves private D-Bus/headless Sway compatibility, not actual
Hyprland focus handoff or multi-process concurrency beyond the current
single-thread desktop event loop.

## Compositor-assigned output criteria versus requested placement

Pinned Mako v1.11 `criteria.c:match_criteria` compares an
`[output=...]` selector with
`notif->surface->surface_output->name`; its `render.c` rematches
criteria immediately before painting, after the compositor can
automatically assign a monitor. The global `output=` config key is
only a layer-shell placement request. Previously Posthorn used that
configured string for `Traits.output`, incorrectly applying a
monitor-specific style before the actual surface had entered any output
and missing output criteria altogether with automatic placement.

Posthorn now keeps the compositor-observed `wl_surface.enter` output
separate from configured placement, invalidates its style revision and
reapplies active grouping when that observed monitor changes.
`LivePresenter.current_output_name` resolves the entered `wl_output`
proxy against the advertised Wayland names. An unmapped surface has
no matching output selector; a remapped surface retains its last known
name until its replacement receives a new enter event, avoiding
temporary style oscillations.

The private test config deliberately omits global `output=`, sends
a scoped category notification, then checks screenshot colors for
the real `HEADLESS-1` output and rejects a deliberately mismatched
`NONEXISTENT` output. Rust/private D-Bus and the 1×, 1.25×, and
2× headless compositor suites passed at
[commit 5fa0e6b](https://github.com/sguzman/posthorn/actions/runs/40005894396).
The 1× live-registry hotplug test also queues a notification for
an absent `HEADLESS-2` monitor, adds that monitor, and requires the
same popup to repaint in a color supplied by a rule scoped to its
**actual new output**, not merely its configured target. The full
private Rust/D-Bus and 1×/1.25×/2× compositor CI passed at
[commit 467e6c1](https://github.com/sguzman/posthorn/actions/runs/40006415764).

Earlier Posthorn revisions used the configured placement to select initial
timeout criteria; a subsequent change correctly used the compositor-entered
output. The regression at
[commit dc26d78](https://github.com/sguzman/posthorn/actions/runs/40006884032)
established the importance of the **observed output**, but its initial
timeout expectation was incomplete. A historical headless regression at
[commit a483bbd](https://github.com/sguzman/posthorn/actions/runs/40008223463)
also passed with that now-superseded assumption. These prior successful
runs are retained as implementation history, **not** as proof of strict
Mako v1.11.0 arrival-time semantics.

Pinned Mako `criteria.c:match_criteria` also requires
`notif->surface != NULL` before either `[anchor]` or `[output]`
can match. On the first Freedesktop `Notify`, the notification's
surface is still NULL when `apply_each_criteria` chooses its timeout
and `on-notify` binding; only **afterwards** does it create or reuse
a surface. Later render passes may apply an output-specific background,
format or visibility rule once the compositor has reported
`wl_surface.enter`, but this must **not retroactively change** the
initial expiration timer. An explicit-ID replacement retains the old
surface pointer through `reset_notification` and may therefore select
surface-specific criteria on arrival. A new stack-tag replacement does
not inherit that surface and starts with NULL.

Posthorn now records per-notification surface-association lifecycle
state, preserving it on direct-ID replacement, clearing it for new/tag
objects and history, and restoring association when history is
reinserted. Rust and private D-Bus regressions cover fresh/direct/tag
`[anchor]` timeout behavior, first versus direct replacement
`[output]` timeouts, and rendering after association. The earlier
headless timeout fixture was revised to require a fresh notification
to remain visible past an output-only 1800ms timeout, followed by
an explicit-ID replacement that must expire under that output rule
and honor `history=0`. Rust unit and private D-Bus validation
passed at [ddd2112](https://github.com/sguzman/posthorn/actions/runs/40038629306);
the corrected Sway acceptance and Rust/private D-Bus suites **both passed** at
[8e7400a](https://github.com/sguzman/posthorn/actions/runs/40038808422).
The subsequent source-evidence commit was also fully green at
[92c8ece](https://github.com/sguzman/posthorn/actions/runs/40039186852),
including 1×, 1.25×, and 2× isolated Sway compositor acceptance.
This proves the bounded NULL-surface and replacement behavior, not
per-notification multi-output association on real Hyprland.

Real Hyprland monitor reassignment, multiple independent notification
surfaces, and complete output-sensitive grouping behavior remain open.

## Touch contact state and notification lifetime safety

The pinned Mako v1.11 `include/wayland.h` tracks at most ten
touchpoints (IDs 0–9). Posthorn's private Wayland state now bounds
its contact map to those IDs, quantizes incoming coordinates at the
same `wl_fixed_to_int` event boundary, tracks independent fingers
through motion and release, ignores unknown releases, and clears
outstanding contacts on cancel or layer-surface replacement. Pure
Wayland-state regressions cover motion, distinct up serials, invalid
IDs, and non-dispatch after cancellation. These changes and the
private D-Bus tests for explicit-ID and app-scoped tag replacement
passed both CI jobs at
[commit 2a24da7](https://github.com/sguzman/posthorn/actions/runs/40013499836).

A touch test that calls the state transition functions is **not** a
real `wl_touch` event delivered by Sway or Hyprland: the headless
wlroots virtual-pointer protocol is pointer-only. Real compositor
touch delivery and physical Hyprland acceptance remain MVP gates.

## Configured sound-hook execution and quiet-mode suppression

Mako uses `on-notify=exec` for configurable notification sound
commands. A new isolated Sway 1× test sends a real Freedesktop
`Notify` into Posthorn's private development session and requires
the configured shell hook to write the exact notification ID from
the `$id` shell variable to a disposable fixture. After activating
quiet mode through the private control D-Bus interface, another
notification of the same category must **not** re-execute that
hook because a `[mode=quiet] on-notify=none` rule overrides it.

Both Rust/private D-Bus and headless Sway 1×, 1.25×, 2× CI jobs
passed at [commit a1565f3](https://github.com/sguzman/posthorn/actions/runs/40024408720).
This verifies **command dispatch and quiet suppression**, not physical
audio playback, PipeWire routing, device selection, audible fidelity,
or the workstation's sound configuration. The test never launches
`pw-play` or touches physical audio. Acoustic acceptance remains open.

After the initial hook has fired, a real private
`fr.emersion.Mako.Reload` must **not** re-execute it for that existing
notification. Likewise, closing the notification and calling the private
`RestoreNotification` history method must restore it without issuing
another shell command. Both isolated compositor and Rust/private D-Bus CI
passed for [reload at commit 9edc1cc](https://github.com/sguzman/posthorn/actions/runs/40026559560)
and [history restoration at commit 6412755](https://github.com/sguzman/posthorn/actions/runs/40026782751).
These test Mako's on-notify arrival semantics, **not physical audio**.

## Included fragment live repaint and invalid rollback

A 1× isolated headless Sway acceptance uses the real Mako `include=`
path to import a disposable `[category=qa-include]` rule, delivers a
real Freedesktop `Notify`, and validates its color in compositor PNG
pixels through the Rust `qa_png_probe`. Editing the included source
and calling `fr.emersion.Mako.Reload` must repaint the **already active**
popup in the new color and remove the previous color. Replacing the
fragment with an invalid color must reject `Reload` and preserve the
last valid rendered popup. This checks actual live compositor behavior,
not just `Config::from_path` unit parsing. The Rust/private D-Bus,
1×, 1.25× and 2× headless Sway CI jobs passed at
[commit 7876ca0](https://github.com/sguzman/posthorn/actions/runs/40028600770).

## Fractional and HiDPI compositor action activation

The separate 1.25× (1536×864 logical) and 2× (960×540 logical) headless
Sway tests now deliver a real virtual wl_pointer **left** click onto a
notification containing a sender-provided default action. Each checks
an additional asynchronous compositor `xdg_activation_token_v1.done`,
extracts the nonempty token and verifies that the exact token is
forwarded on the private D-Bus `ActivationToken` signal **before**
`ActionInvoked` and `NotificationClosed`. A real compositor PNG
must also show dismissal of the targeted popup while older messages
remain visible. Both scales passed at
[commit 7876ca0](https://github.com/sguzman/posthorn/actions/runs/40028600770).
This is evidence of scaled **logical pointer routing and XDG token
handshake**, not real Hyprland focus activation, physical input,
fractional protocol buffer negotiation, touch, or installed Mako cutover.

## Explicit nonclaims

Normal development commands register only the **private development D-Bus name**; standard Freedesktop registration is exercised solely in an isolated CI session.
It is not the workstation's default desktop notification service. The private desktop path
renders and updates stacked Wayland popups, with basic mouse/touch bindings,
with layer-shell configure/SHM attachment and success, failure, and warning screenshot colors, newest-first stacking, right-anchor placement, post-CloseNotification popup removal, timed expiry/repaint under isolated 1x/2x headless Sway, and dynamically added/removed named output recovery under isolated 1x Sway; it has not been visually exercised under Hyprland. Exact Mako Pango metrics,
replacement-time grouping edge cases, live Hyprland monitor hotplug, exact fractional-buffer behavior, real Hyprland activation/focus and touch QA,
complete mode/reload regrouping semantics and control formatting remain. Existing active timeout deadlines are preserved across reloads, as in the pinned Mako source.
Current parsing is a subset; remaining POSIX vs Rust regex differences and many style options
still need parity tests. The
presence of a config key in a map does not mean the visual behavior is ported.
No MVP/release or workstation cutover until the end-to-end acceptance suite
passes. The README and progress report must not call this a working Mako port.

## Group count format placeholder signedness

Pinned Mako v1.11.0 declares `mako_notification.group_count` as C `int`
(`include/notification.h`) and formats `%g` using `mako_asprintf("%d",
notif->group_count)` (`notification.c`). Posthorn now narrows its internal
`usize` count to signed 32-bit before formatting, as it already does for
Mako's signed `%i` notification ID. Synthetic Rust regressions exercise
zero, `INT_MAX`, `INT_MAX + 1` and `UINT32_MAX` values; the complete
private D-Bus and 1×/1.25×/2× isolated Sway suite passed in
[run 40030597962](https://github.com/sguzman/posthorn/actions/runs/40030597962)
at commit `7bd46a5`. This is source-level placeholder compatibility,
not evidence of real desktop grouping under billions of notifications.

## Zero-valued scoped max-visible validation

Mako v1.11.0 `criteria.c:validate_criteria` checks the **numeric**
`criteria->style.max_visible` field, not its style-spec presence bit,
when deciding whether non-surface selectors may specify max-visible.
Consequently `[category=urgent] max-visible=0` is valid, while the
same selector with any nonzero value is rejected. `types.c:parse_int`
uses `(int)strtol` on 64-bit Linux, so values like `4294967296`
narrow to zero and follow the same acceptance branch. Criteria are
validated at section end using the final assignment.

Posthorn now validates the narrowed final numeric value and has source-pinned
Rust regressions for zero, signed zero, narrowing, nonzero, overwritten
assignments and surface-scoped negative values. The isolated Rust/private
D-Bus and 1×/1.25×/2× Sway CI completed successfully at
[commit d96b461](https://github.com/sguzman/posthorn/actions/runs/40031933688).
This is configuration acceptance evidence, **not** physical Hyprland or
production cutover acceptance.

## Deferred history pruning after max-history reload

Pinned Mako v1.11.0's `config.c:reload_config` and
`dbus/mako.c:reapply_config` leave the history list untouched. Its
`notification.c:close_notification` enforces the then-current
`max_history` only when appending a newly closed notification to
history. Consequently, lowering `max-history` from 3 to 1 retains
three earlier entries until the next eligible dismissal. Reloading
to zero retains old history, while future closes do not append.

Posthorn now mirrors those retention boundaries. Rust lifecycle and
private D-Bus `Notify`, `DismissNotifications`, `Reload`, and
`ListHistory` regression cases cover 3 → 1 → 0 transitions.
The Rust tests, private D-Bus suite, and isolated Sway tests at
1×, 1.25×, and 2× passed for
[commit a88bb04](https://github.com/sguzman/posthorn/actions/runs/40032485795)
(also [preceding store fix](https://github.com/sguzman/posthorn/actions/runs/40032406098)).
Real Hyprland and production bus behavior remain unverified.

## Stack-tag replacement ID allocation order

Pinned Mako v1.11.0's `dbus/xdg.c:handle_notify` calls
`notification.c:create_notification` when no active explicit
`replaces_id` is found, **before** it tests stack-tag matching via
`get_tagged_notification`. A matched tag replaces the new object's
public ID with the existing notification's ID, but the newly consumed
`state->last_id` is not rolled back. By contrast, replacing an active
explicit ID resets that notification in place and consumes no new ID.

Posthorn now consumes the intermediate ID for each stack-tag replacement.
Source-pinned unit and private development D-Bus regressions verify two
tagged updates, the next ordinary notification ID, direct-ID replacement,
and unmatched replacement ID handling. CI run
[40034166141](https://github.com/sguzman/posthorn/actions/runs/40034166141)
passed 257 Rust unit tests, 52 private D-Bus integration tests, and
isolated 1×, 1.25×, and 2× Sway compositor acceptance.
This does **not** establish real workstation Hyprland parity or authorize
production notification service ownership.

## Preserved group index when replacing a live notification

In pinned Mako v1.11.0, `notification.c:reset_notification` clears the
old payload and timer but preserves `group_index` on a direct
`replaces_id` update. `dbus/xdg.c:handle_notify` calls
`apply_each_criteria` before regrouping, so `[group-index=1]`
timeout criteria apply to the preserved index at scheduling time.
A stack-tag replacement instead creates a fresh ungrouped object and
does **not** inherit the former index for timeout selection.

Posthorn's source-pinned unit regressions cover both paths; the private
D-Bus regression verifies the indexed timeout through the real
`Notify` method on `org.posthorn.Development`. Both CI jobs passed
for [commit 50a15c5](https://github.com/sguzman/posthorn/actions/runs/40036657372).

## Selecting on-notify bindings before notification regrouping

Pinned Mako v1.11.0 `dbus/xdg.c:handle_notify` selects the binding
when `apply_each_criteria` applies the notification style **before**
`group_notifications`, then runs that stored binding after grouping
without reapplying criteria. Thus, a newly formed group cannot
retroactively select a `[grouped] on-notify` hook. A direct-ID
replacement can select it when its preserved index was already grouped;
a stack-tag replacement starts ungrouped. Restoring history does not
replay `on-notify` hooks.

Posthorn now stages the selected binding once per new Notify, dispatches
it after grouping, and discards the staged value when consumed or when
the notification is closed/expired. The previous integration test was
corrected to check pinned Mako semantics rather than expecting a hook
from the newly grouped style. Unit and private D-Bus integration tests
passed on [commit 30c2562](https://github.com/sguzman/posthorn/actions/runs/40037106582):
260 unit tests, 54 private D-Bus tests. The earlier code-only commit
[503fa7e](https://github.com/sguzman/posthorn/actions/runs/40036902279)
passed isolated 1×, 1.25×, and 2× Sway compositor acceptance; its
older integration suite included the corrected historical expectation.
A later full CI run also passed on
[92c8ece](https://github.com/sguzman/posthorn/actions/runs/40039186852),
including this regression and isolated compositor checks.

These checks do not authorize production D-Bus ownership or establish
real Hyprland pointer, touch, compositor or cutover acceptance.

## Category-selected surface anchor preserved on explicit ID replacement

Pinned Mako v1.11.0 `criteria.c:match_criteria` evaluates
`[anchor=...]` against `notif->surface->anchor`, not the root
`anchor=` configuration. A first notification can select a non-root
anchor through a category rule. On direct `replaces_id`, Mako
`notification.c:reset_notification` retains that surface association;
its next pre-regroup criteria pass can therefore match the **old
surface anchor** even when the new notification no longer belongs to
the old category. New notifications and app-scoped stack-tag
replacements start without an associated surface.

Posthorn now retains the chosen anchor alongside surface-association
state, preserves it for direct replacements, and avoids borrowing it
for new/tagged objects or persisted history. Isolated unit and
Freedesktop Notify/private D-Bus regressions cover a category-specific
bottom-left surface followed by a direct replacement with a different
category; the replacement chooses the old anchor's timeout instead
of the root timeout. Both jobs, including 1×, 1.25×, and 2× isolated
Sway compositor acceptance, passed at
[commit 0b5bb43](https://github.com/sguzman/posthorn/actions/runs/40040039068).

This is bounded Mako parity for **retaining the existing surface
anchor**. Reassignment between multiple currently live surfaces,
per-notification real-output identities, and physical Hyprland
acceptance are still outstanding; no production Mako cutover occurred.

## Reapplying category surface anchor on history restoration

Pinned Mako v1.11.0 `dbus/mako.c:handle_restore_action` removes
the historical notification from history, calls `finish_style` and
`init_empty_style`, applies current criteria while the notification's
surface is still NULL, inserts the notification in active sort order,
and marks its selected surface dirty. It neither runs `on-notify`
nor creates a new expiration timer.

Posthorn now repeats the criteria-based anchor association on the
restored active notification before reapplying history policy. In
particular, `[category=warning] anchor=bottom-left` can choose the
restored surface even when the root anchor is top-right and the
historical object's surface pointer was cleared on close.
The existing non-rearmed timeout and no-hook behavior remain intact.

CI [40040434719](https://github.com/sguzman/posthorn/actions/runs/40040434719)
passed 263 Rust unit tests, 57 private D-Bus tests and all isolated
1×, 1.25×, 2× Sway compositor acceptance. This is bounded restored
anchor parity; real multi-surface output reassignment and physical
Hyprland acceptance remain unverified. The user's installed Mako,
autostart, and production D-Bus ownership are untouched.

## Existing surface tuple matching and recomputation on mode changes

Pinned Mako v1.11.0 `criteria.c:apply_each_criteria` searches its
persistent `state->surfaces` for an existing surface with the newly
selected **configured output, layer and anchor**. If one matches, the
notification switches association; if not, a notification with an
existing surface keeps it, and a notification with NULL surface creates
one. The search runs after initial style and timeout selection. Surface
identity is independent of the configured monitor's eventual
`wl_surface.enter` output.

Posthorn now records those reusable identity triplets. A direct-ID
replacement can relocate to a previously created matching surface,
but a different layer or configured output does not qualify; its
initial timeout is still based on its old association. Focused Rust and
private D-Bus regressions exercise this behavior. Both isolated CI
jobs were green at
[4c834d4](https://github.com/sguzman/posthorn/actions/runs/40042134097).

Pinned `dbus/mako.c:reapply_config` destroys *all* old surfaces,
clears each active notification's surface and group-index, reapplies
styles, and rebuilds associations in active list order. Posthorn
now invalidates its identity registry and active surface associations
on mode/config reapplication, regenerates surface identities alongside
grouping, and preserves existing expiration deadlines. Unit and
private D-Bus mode-transition regressions prove that a mode may move
an active notification from top-right to bottom-left and back without
retaining a stale tuple or rearming the timer. Both CI jobs passed at
[d1c2df3](https://github.com/sguzman/posthorn/actions/runs/40042280919).

## Per-notification output selectors and output-change history

Mako's `criteria.c:match_criteria` compares `[output=...]` against
the particular `notif->surface->surface_output` supplied by the
compositor, not a shared daemon-wide output. Posthorn now supports
output observations scoped to a notification's associated Mako surface
identity (including an explicitly unmapped state). The existing single-surface private
presenter continues to report its single observed output as a
fallback for notifications with no independent observation. The
specific observation is retained on direct-ID replacement and cleared
on new/tag objects and mode/config surface reconstruction. Rust and
private D-Bus tests verify two notifications with distinct outputs
can match distinct `[output]` criteria and that replacement selects
its own surface's pre-regroup timeout. Both CI jobs passed at
[1d1106c](https://github.com/sguzman/posthorn/actions/runs/40042525507).

Mako's `render.c` reapplies criteria after compositor assignment.
Consequently, output changes can affect active `history` settings
without retroactively rescheduling expiration timers. Posthorn now
refreshes its active history-policy map on single-surface
`wl_surface.enter`/leave observations as well as independently
observed notification output changes. The last acceptance suite passed
**267 Rust unit tests, 61 private D-Bus tests**, and isolated
1×/1.25×/2× Sway compositor tests at
[01334a3](https://github.com/sguzman/posthorn/actions/runs/40042641411).

**Limitations:** This is surface identity, criteria, and state-model
parity; the current private live presenter still maps one layer-shell
notification surface, and the new per-notification observation API
is not a claim that it currently maps or renders multiple independent
real surfaces. Output hotplug/reassignment across truly simultaneous
surfaces and physical Hyprland multi-monitor QA remain outstanding.
No production D-Bus ownership, installed Mako, or desktop autostart
was changed.

## One compositor output per shared Mako surface

Pinned Mako v1.11.0 `criteria.c:apply_each_criteria` associates
multiple notifications with **one** `mako_surface` when their selected
configured-output/layer/anchor tuples are identical.
`wayland.c:surface_handle_enter` stores the compositor-entered output
on that `mako_surface` itself. Its `surface_handle_leave` clears it.
Consequently, two notifications that share a tuple **cannot**
simultaneously observe distinct compositor outputs. An earlier Posthorn
test modeled that impossible situation, despite green CI.

Posthorn now retains the exact surface tuple on each active
notification. Updating an entered/left output for one notification
updates all active notifications associated with that same surface.
Direct-ID replacement preserves the tuple; fresh notifications can
join an existing compatible surface and inherit its observed output;
reassignment between different tuples carries the target surface's
observation; stack-tag replacements and archived history do not retain
the previous surface association. Mode and config reapplication
clear the surface tuples and reconstruct them.

The independent-output test fixtures were corrected to use two
**genuinely distinct surface tuples**. A new regression verifies that
two notifications sharing one tuple both track the same output, and
that a direct-ID replacement retains it. Another source-aligned
regression simulates a new `wl_surface.enter` after a previously
unmapped tuple becomes visible again, before expecting output-scoped
replacement timeout selection. Both CI jobs passed at
[7a86a43](https://github.com/sguzman/posthorn/actions/runs/40043446892):
**268 Rust unit tests, 61 private D-Bus tests**, plus isolated
1×, 1.25× and 2× Sway compositor checks.

These verified invariants are **not** yet proof of a full multi-layer
or multi-output live renderer. The development presenter still maps a
single layer-shell notification surface. Its replacement must support
actual simultaneous compositor surfaces and separate enter/leave
callbacks before any corresponding 100% acceptance claim. The user's
installed Mako, live Hyprland session and production D-Bus owner remain
untouched.

## Independent max-visible counting per associated surface

Pinned Mako v1.11.0 `render.c` loops over each surface separately,
ignores notifications where `notif->surface != surface`, and starts
`visible_count` from zero for each surface. In particular,
`max-visible=1` allows one eligible group on a top-right surface
**and another** eligible group on a bottom-left surface. A daemon-wide
counter incorrectly suppresses one of them.

Posthorn now scopes its projected visible-group count by the stored
(configured output, layer, anchor) surface tuple, instead of using
one cross-surface count. Focused Rust and private D-Bus tests send
multiple notifications into two distinct surface identities and verify
that a newer top-right notification hides an older one while a
bottom-left notification remains eligible. Both CI jobs passed at
[0d8e8c5](https://github.com/sguzman/posthorn/actions/runs/40043801396):
**269 unit tests, 62 private D-Bus tests**, plus 1×, 1.25× and 2×
isolated Sway compositor acceptance.

The synthetic hidden-count placeholder remains aggregated in the
single-surface frame path, and the live presenter still draws one
layer-shell popup. Actual independent surface geometry, per-surface
hidden placeholders and simultaneous multi-output painting remain
unverified and are not claimed complete. Production Mako, Hyprland
configuration and the real notification bus were not changed.

## Per-surface projection, rasterization, hit-testing, and two live popups

Pinned Mako v1.11.0 `render.c` renders each `mako_surface`
independently, counting total notifications, visible groups, and
overflow notifications **within that surface**. `wayland.c` creates
one layer-shell surface for every active output/layer/anchor surface
identity and receives separate `wl_surface.enter` output callbacks.

Posthorn now provides `surface_visibility` with ordered
notification IDs, total counts and hidden counts per matching surface
identity, verified at
[de6ad42](https://github.com/sguzman/posthorn/actions/runs/40044220460).
The new `snapshot_surface_frame_scaled` rasterizer draws only one
surface's notification stack and its own `%h` overflow placeholder.
A Rust unit and private D-Bus regression use two differently anchored
surfaces and confirm that foreground and overflow colors do not bleed
between them; CI passed **271 Rust unit and 64 private D-Bus tests**
at [1678886](https://github.com/sguzman/posthorn/actions/runs/40044473233).
`hit_test_on_surface` likewise uses the correct isolated surface
coordinates, notification subset and anchor-specific stack alignment.
Its regression uses measurable non-empty popup content; CI passed at
[04f063d](https://github.com/sguzman/posthorn/actions/runs/40044745049).

Two acceptance levels are distinguished:
1. A standalone Wayland probe maps independent top-left and
   bottom-right layer-shell popups on separate client connections in
   isolated headless Sway. The compositor screenshot contains both
   probe colors simultaneously.
2. An explicitly opt-in Posthorn
   `--experimental-private-multi-desktop PATH` mode maps each
   notification surface identity using a separate `LivePresenter`,
   compositor-output observation, SHM buffer lifetime and hit-test
   coordinate system. The test sends two actual Freedesktop
   `Notify` calls over **org.posthorn.Development**, targeting
   separate anchors, and confirms both notification colors in
   the **same headless Sway screenshot**.

Both jobs passed on
[cf1dac6](https://github.com/sguzman/posthorn/actions/runs/40044998721):
**272 Rust unit tests, 65 private D-Bus tests**, and existing
single-surface 1×/1.25×/2× headless Sway acceptance. The new
simultaneous multi-surface screenshot check was executed at 1×,
not at fractional or HiDPI scales.

The original `--experimental-private-desktop` single-surface
rendering path remains intact for regression coverage. This evidence
supersedes the earlier limitation that Posthorn had only a single
live presenter: the separate **experimental** renderer now maps
multiple real layer-shell surfaces in isolated Sway. It does **not**
verify multiple physical outputs, output migration/hot-unplug under
the multi-renderer, simultaneous multi-surface pointer input on
physical Hyprland, or the reversible live Mako replacement gate.
The user's installed Mako, Hyprland/autostart and real notification
D-Bus ownership remain untouched.

## Simultaneous multi-surface acceptance at every isolated scale

CI [40045421933](https://github.com/sguzman/posthorn/actions/runs/40045421933)
passed all of the original 1×, 1.25×, and 2× Sway compositor
acceptance gates **including** the opt-in private multi-surface
D-Bus notification screenshot checks at each scale.
The public Freedesktop notification bus was not acquired; all of these
checks used an ephemeral D-Bus session and `org.posthorn.Development`.

The same CI run launched a **separate two-monitor** Sway instance
with virtual `HEADLESS-1` and `HEADLESS-2` outputs. Config selected
`output=HEADLESS-1` globally and
`[category=second] output=HEADLESS-2`. Notifications were delivered
through private `Notify` calls. The `grim` screenshot for each named
output showed **only** the intended surface's notification color;
the opposite surface color was absent. The runner then unplugged
`HEADLESS-2` and verified that Posthorn's private D-Bus service
remained operational while `HEADLESS-1` retained its notification.

The headless CI run passed **272 Rust unit tests, 65 private D-Bus
tests**, three scale-specific compositor checks, and the independent
two-output placement/unplug gate. These checks exercise real
Wayland protocol objects and screenshots with isolated virtual
monitors; they do **not** establish physical Hyprland acceptance,
multi-surface input delivery across physical monitors, dynamic
replug onto a differently named output, Edge-originated activation
tokens, or the expressly opt-in Mako cutover. Accordingly, T401 stays
under 100%, the workstation/end-to-end gate T503 advances only
incrementally, and T504 remains 0%. Existing Mako, autostart, and
the production notification bus are untouched.

## Pointer events stay on the selected compositor output

Pinned Mako v1.11.0 `wayland.c` resolves pointer events from the
underlying Wayland surface, not a global concatenated screen geometry.
Posthorn's opt-in multi-surface desktop now gives every independently
mapped output/layer/anchor surface a dedicated compositor input stream
and uses `hit_test_on_surface` to resolve local coordinates.

The isolated two-virtual-output Sway acceptance positions
`HEADLESS-1` at (0,0) and `HEADLESS-2` at (1280,0).
After matching each surface's notification background color in its
own screenshot, the test sends an actual compositor virtual-pointer
right-click inside the second output's notification. It then verifies
that the second notification's color is **absent** from the second
output while the first output's notification color remains **present**.
The test additionally removes `HEADLESS-2` and verifies the daemon
remains registered on the isolated D-Bus name and still displays the
survivor on `HEADLESS-1`.

CI [40045890584](https://github.com/sguzman/posthorn/actions/runs/40045890584)
passed both jobs: **272 Rust unit tests, 65 private D-Bus
tests**, the three isolated display-scale suites, and the
two-output/surface-local pointer/dismissal/unplug acceptance. This
establishes scoped input delivery in disposable Sway, **not** a
physical Hyprland pointer/touch/XDG activation-token acceptance or
permission to replace a user's installed Mako. Consequently T403
and T503 advance only slightly; T504 remains 0%.

## Mixed layer-shell layers and symmetric pointer routing in isolated Sway

Mako v1.11.0 `criteria.c:apply_each_criteria` includes the selected
layer in the reusable surface identity, and `wayland.c` maps that layer
with `zwlr_layer_shell_v1_get_layer_surface`. Posthorn's opt-in
`--experimental-private-multi-desktop` now has acceptance evidence
for a default `top` notification and an independently anchored
`overlay` notification **simultaneously visible** at each isolated
1×, 1.25× and 2× Sway scale. Both jobs passed at
[run 40046442239](https://github.com/sguzman/posthorn/actions/runs/40046442239).

A later two-headless-monitor regression exercises real compositor
pointer input both ways: clicking unused space on `HEADLESS-2`
must keep both notifications; right-clicking the notification on the
second output must leave the first notification mapped; sending
a new second-output notification must remap that popup; then clicking
the first output must preserve the second. Finally the first popup
is restored before unplugging the second monitor. The compositor
screenshots assert the surviving notification color and the absence
of only the dismissed notification color at each transition.

[CI run 40046494158](https://github.com/sguzman/posthorn/actions/runs/40046494158)
passed **272 Rust unit tests, 65 private D-Bus tests**, all three
Sway scale suites, and the two-output gap-click, bidirectional
dismissal, remapping and unplug acceptance. All runs use the
private test D-Bus name, never the user's desktop notification owner.
This does not prove real Hyprland focus/activation parity or
authorize production Mako replacement.

## Exact icon-to-text offsets and orthogonal text centering

Pinned Mako **v1.11.0** `render.c:render_notification` places text
alongside a left icon at `icon.width + 2*padding.left` (relative to
the border) and below a top icon at `icon.height + 2*padding.top`.
It then vertically centers Pango text for horizontal icon locations,
including a notification without an icon, or horizontally centers it
for vertical icon locations. Posthorn previously added **one extra
padding unit** to the left/top text origin and left short text aligned
to the top of tall left/right icons, or at the left edge below top
icons.

[Commit 9598a9a](https://github.com/sguzman/posthorn/commit/9598a9a92452a2e98f96c77e1e0531cd89a3dc00)
corrects those origin calculations and measures the Pango layout
before deciding the final orthogonal text offset. A dedicated
`pinned_mako_icon_text_offsets_and_axis_centering` regression
renders actual icon pixels and white Pango glyphs. It checks that
12-pixel left/top icons with four-pixel padding displace the glyph
by **exactly 16 pixels** along the icon's axis; it also checks
vertical centering alongside a 28-pixel-tall icon and horizontal
centering below a top icon.

[CI run 40056468251](https://github.com/sguzman/posthorn/actions/runs/40056468251)
passed both the Rust/private-D-Bus job and the isolated headless
Wayland job, including the existing 1×, 1.25×, 2×, dual-output,
input-region, mixed-scale, and signed-padding gates. The new glyph
offset assertions are **unit-level rendered-pixel checks**, not a
new screenshot comparison against upstream Mako or a physical
Hyprland test. Accordingly T105 advances from 81% to **82%**,
while the M0 milestone remains **88%**, T503 remains 51% and
the unauthorized production cutover T504 remains 0%.
The real notification session and installed Mako were not touched.

## Truncated fractional icon dimensions and source-matched icon positioning

Pinned Mako **v1.11.0** `icon.c:create_icon` calculates
`fit_to_square` and assigns scaled `icon->width` and
`icon->height` into `int` fields, **truncating** fractional
dimensions. `render.c:render_notification` uses those truncated
sizes to reserve Pango text space and draw the icon clipping
rectangle. It does not shrink the image again to fit the remaining
notification height or text area. A top/bottom icon is centered at
`(notif_width - icon->width) / 2`, independently of asymmetric
text padding.

Posthorn previously **rounded** both scaled dimensions, then
independently reduced the scale in the painter to fit the
shrink-wrapped content area. The result could paint an icon narrower
than Mako's selected size, mismeasure popup height by one pixel
and shift a centered vertical icon when horizontal text insets were
asymmetric.

[Commit 000140c](https://github.com/sguzman/posthorn/commit/000140c47cbf9972e5cbe671a362001c5ae04aca)
corrected integer dimension truncation, applied the same icon scale
to measurement and painting, clips to Mako's truncated dimensions,
and centers top/bottom icons without text padding. The new unit
regressions use a **12×7** red bitmap constrained by
`max-icon-size=8`, requiring an **8×4** logical clipping
rectangle. They also verify the leftmost and rightmost icon pixels
and top-centered placement under nonuniform insets.

The initial test commit used exact 255-channel Cairo pixel asserts;
the rasterizer emitted 254 on fractionally sampled red edges,
without changing the geometric boundary. The test-only correction
[0e2ca7b](https://github.com/sguzman/posthorn/commit/0e2ca7bf075e0852845706902f35c5eca2df1365)
accepts that **one-level** premultiplied color rounding while still
checking exact transparent/black bounds and solid alpha.

[CI 40057301556](https://github.com/sguzman/posthorn/actions/runs/40057301556)
passed **both jobs**: Rust unit and private D-Bus suites,
plus the 1×, 1.25×, 2×, dual-output, input-region, mixed-output
DPI, and signed-padding disposable Sway gates. The new bitmap
truncation and position checks were rendered-pixel unit tests;
there has not been a side-by-side photographic oracle against
upstream Mako on physical Hyprland.

T105 advances to **83%**; P1 remains **89%** and M0 stays
**88%** because the remaining acceptance gates are independent.
T503 remains 51% and T504 remains 0%. Neither installed Mako
nor production Wayland/D-Bus state was modified.

## Config superstyle outer margins include even unmatched criteria

Pinned **Mako v1.11.0** `config.c:apply_superset_style` starts from an
empty style with signed zero values and calculates the **componentwise
maximum** `outer_margin.top/right/bottom/left` across the root and
every configured criteria style. `wayland.c:send_frame` sends that
`config.superstyle.outer_margin` to `zwlr_layer_surface_v1_set_margin`.
It does **not** use only the root `outer-margin`, and the matching
notification's ordinary `margin` does not change that layer-shell
property.

Posthorn had instead read `config.global["outer-margin"]` only.
This lost surface placement in configurations specifying
`outer-margin` under category, output or mode sections, even when
those criteria did not match the current notification.

[7119590](https://github.com/sguzman/posthorn/commit/7119590d5eab31ec34e98a01e641a7da10c19664)
corrected `desktop::outer_margins` to take the signed per-edge max
across the root and **all** user criteria (anchored at zero, as in
Mako). A Rust regression verifies two different unmatched criteria
can independently determine the final top, right, bottom and left
values; negative-only settings cannot decrease the zero-initialized
superstyle, and notification `margin` cannot replace `outer-margin`.
The compositor's existing single- and multi-surface presenters use
this resolved value.

[4196ccb](https://github.com/sguzman/posthorn/commit/4196ccb89cd742cc68b887be64fdef1b6724f7ea)
added an **actual isolated Sway screenshot** acceptance gate.
A private D-Bus notification does not match either configured
category, but the popup must still land **85 logical pixels from the
right edge** and **32 pixels from the top** because the two
unmatched criteria each contribute an `outer-margin` component.
The compositor screenshot's color bounds matched those exact
positions.

[GitHub Actions 40113437561](https://github.com/sguzman/posthorn/actions/runs/40113437561)
passed both the Rust/unit/private-D-Bus test job and the disposable
headless Wayland job, including this new screenshot assertion,
the independent two-output, mixed-DPI, pointer, and signed-padding
gates. The previously established live Mako and physical Hyprland
acceptance restrictions remain unchanged. **T105 83% → 84%**,
**P1 stays 89%**, **M0 stays 88%**; no post-MVP work was started
and no installed compositor, Mako service or real D-Bus owner changed.

## Private compositor transactional configuration reload and geometry restoration

Pinned Mako v1.11.0 `dbus/mako.c:handle_reload` calls
`reload_config` and `reapply_config` for a successful reload; an
invalid configuration produces `fr.emersion.Mako.InvalidConfig`
without changing the prior active configuration. The earlier
Posthorn source parity work proved that the configured layer-shell
`outer-margin` is the componentwise maximum across **all** criteria,
including unmatched ones.

[Commit 83a26f3](https://github.com/sguzman/posthorn/commit/83a26f3a69dc49f507b11067ab7fe67d6a710d6e)
extends the existing disposable private Sway + private D-Bus
`headless_superstyle_margin.sh` acceptance suite. It retains
**one existing notification ID** and takes real compositor screenshots
through this sequence:
1. Start with unmatched criteria contributing top 32/right 85;
2. Rewrite those same criteria to top 12/right 40, invoke Mako `Reload`,
   and require the **existing** popup to relocate correctly;
3. Insert an invalid option in the config, require a rejected D-Bus
   `Reload` response, and verify top 12/right 40 is preserved;
4. Restore the valid file, invoke `Reload` and require the existing
   popup to return to top 32/right 85 without sending another Notify.

[CI 40118186038](https://github.com/sguzman/posthorn/actions/runs/40118186038)
passed both the Rust/private-D-Bus tests and the full disposable
headless Sway job including this **runtime screenshot** test.
This verifies rollback behavior and geometry updates on the
development name and compositor only. No claim about physical
Hyprland, the real org.freedesktop.Notifications owner, or production
Mako replacement follows from it.

P3/T303 advances **77% to 78%**; the P3 phase remains **91%**
and M0 remains **88%** on their unchanged source-backed ledger.
All prior unaccepted physical and cutover gates remain open.

## Direct Notify replacement sound hooks and quiet-mode reactivation

Pinned Mako **v1.11.0** `dbus/xdg.c:handle_notify` resolves and
updates an existing notification when `replaces_id` is active, and
still invokes `notification_execute_binding` for its effective
`on-notify` style after grouping. `mode=quiet` can override that
binding to `none`, suppressing notification sounds without dropping
the notification. Mode changes themselves must not replay hooks.

[Commit 060111a](https://github.com/sguzman/posthorn/commit/060111ad1659a69a9a8aedf717615dded03d4479)
extends the existing **private D-Bus + disposable Sway** sound-hook
acceptance at scale 1× with a real shell-command output file, not
a physical audio device. The test delivers a categorized notification
and waits for its `on-notify=exec` ID, then sends a second Notify
with the **same active replaces_id**, verifies the returned ID is
unchanged, and requires exactly one new hook invocation containing
that original ID. It switches to quiet mode, replaces the notification
again, and asserts no hook runs. Restoring default mode by itself
must not execute a hook; one more replacement must resume the hook,
again using the unchanged notification ID. The fixture is dismissed
at the end, entirely within the isolated service.

[GitHub Actions 40119691973](https://github.com/sguzman/posthorn/actions/runs/40119691973)
passed both the Rust/unit/private-D-Bus job and the headless Wayland
job including that full replacement/quiet lifecycle. This is
accepted **hook/lifecycle parity**, not physical speaker, PipeWire or
Hyprland sound QA; no Mako production process or user's session bus
was accessed. **T502 advances 75% → 76%** and the separate Mako
category/sound feature inventory to **76%**. The P5 average becomes
**56%**, while the overall M0 milestone remains **88%**, T503 remains
51% and T504 remains 0%.

## Replacement expiration cancels the original deadline on live compositor

Pinned Mako **v1.11.0** `dbus/xdg.c:handle_notify` calls
`reset_notification` when `replaces_id` refers to a live
notification. This destroys its old event-loop timer; after resolving
the replacement's effective style it schedules a **new** timer from
that payload's `expire_timeout`. The old deadline must not close the
replacement, and the new timer must expire the retained ID and
move only the replacement payload into history when eligible.

[Commit e22102d](https://github.com/sguzman/posthorn/commit/e22102d66f974db337ffed2dc67d52bb3cd090ac)
adds a **private D-Bus + isolated headless Sway** acceptance sequence
at scale 1×. It submits a warning notification with a 1200ms sender
timeout; replaces it after 200ms using the same Freedesktop ID and a
3000ms timeout; observes that the new summary and real popup survive
beyond the first deadline; then waits for expiration of the second
deadline, checks the notification vanished from the active list, and
confirms that history contains only the replacement summary.

[CI 40120085647](https://github.com/sguzman/posthorn/actions/runs/40120085647)
passed both Rust/private-D-Bus and disposable headless Wayland test
jobs, including this new live expiration sequence. The deterministic
unit scheduler had already covered cancellation on ID replacement;
the new acceptance verifies the actual runtime, D-Bus and compositor
integration as an independent gate.

**T203 advances 97% → 98%**. P2 remains **96%**, M0 remains
**88%**, T503 remains 51% and T504 remains 0%. This was run only on
the development D-Bus name and disposable Sway output, never on
the live Mako daemon or workstation session.

## Private D-Bus Pango body-markup acceptance

Pinned Mako v1.11.0 `notification.c:format_text` validates sender Pango body markup
before embedding it in formatted text; invalid sender markup is escaped.
The isolated regression in `tests/private_markup.rs` sends a valid red
foreground span over private D-Bus, verifies that the original `Notify`
body survives and renders red glyph ink via Cairo/Pango, and checks that
`markup=0` leaves the sender's tags non-operative. It also delivers a
malformed body and verifies that rendering succeeds rather than dropping
the notification. The accepted test counts saturated red pixels so that
subpixel-antialiasing fringes in uncolored text do not cause false positives.

Both Rust/private D-Bus tests and isolated headless Sway acceptance
(1×, 1.25×, 2×, multi-output and pointer regressions) passed at
[commit 1aab5af](https://github.com/sguzman/posthorn/actions/runs/40121480856).
This does not prove physical Hyprland acceptance, full T205 parity, or
readiness to switch the production notification daemon.

## Private Sway acceptance of three category sound hooks and quiet mode

Pinned Mako **v1.11.0** `config.c:apply_style_option` accepts trusted
per-category `on-notify=exec` bindings; `dbus/xdg.c:handle_notify`
executes the effective notification binding after applying criteria.
Later `[mode=quiet]` criteria can override the configured category
binding to `none`.

[Commit b51e345](https://github.com/sguzman/posthorn/commit/b51e345817415281c3189cdcc73d9ad920f30ff4)
extends the existing **isolated headless Sway 1×** acceptance harness
with three separate outcome categories: `success`, `failure`, and
`warning`. Each private D-Bus `Notify` must invoke only its own
trusted shell-log hook, exactly once and with its returned Freedesktop
notification ID, in the correct delivery order. A config reload must
not replay any hook. After `SetMode quiet`, newly delivered
notifications in **all three categories** must leave the original log
unchanged; restoring the default mode must likewise not replay hooks.
Every fixture is explicitly closed over the private service.

[GitHub Actions run 40124663468](https://github.com/sguzman/posthorn/actions/runs/40124663468)
passed both jobs, and its headless Wayland log explicitly printed
`Private success/failure/warning hook routing and quiet suppression: OK`.
The other 1.25×, 2×, multi-output, input-region, mixed-DPI, superstyle
and signed-padding regressions remained green.

**T502: 76% → 77%** and category/sound feature inventory **76% → 77%**;
P5 stays **56%** and strict M0 stays **88%**. This is automated
**configured command-dispatch acceptance**, not physical playback,
PipeWire volume, real Hyprland acceptance, or production cutover.
No real Mako process, notification bus owner, or user configuration
was accessed.

## C-locale POSIX ERE byte matching in notification criteria

Pinned **Mako v1.11.0** `criteria.c` compiles summary and body
selectors with `regcomp(REG_EXTENDED | REG_NOSUB)` and matches using
`regexec`. The process does not call `setlocale`, so the initial
C locale's character classes and regex dot operate on bytes, not
Unicode code points. A source-aligned glibc probe confirmed that
`^[[:alpha:]]+$`, `^[[:alnum:]]+$`, and GNU `^\\w+$`
reject non-ASCII accented/CJK text in this locale; one POSIX dot
also does not cover a two-byte UTF-8 `é`.

Posthorn previously compiled these expressions as Unicode-aware
`regex::Regex`. [Commit e078cc7](https://github.com/sguzman/posthorn/commit/e078cc704938871d6eae83c62e07b1c0a5c8e654)
switches to the **pure-Rust** `regex::bytes::Regex` engine with
Unicode mode disabled and dot-all enabled. This preserves the
existing C-locale POSIX escape normalization and corrects ASCII
classes and UTF-8 byte-width comparisons. Focused unit tests
check both positive ASCII and negative accented/CJK examples,
plus one- versus two-dot matching of `é`.

The private D-Bus regression uses the actual Mako criteria tokenizer,
which removes one layer of backslash escaping, then delivers
ASCII and non-ASCII summaries/bodies via `Notify` to confirm
which `on-notify=dismiss` rule executes. Initial fixture
failures involved an underscore in a POSIX `alnum` sample
and an incorrectly single-escaped config `\\w` expression;
those fixture errors were corrected rather than counted as
implementation parity.

[CI 40126481126](https://github.com/sguzman/posthorn/actions/runs/40126481126)
passed both the Rust/private D-Bus test job and disposable headless
Sway acceptance, including scaling and output regressions, on
[commit a19b13c](https://github.com/sguzman/posthorn/commit/a19b13ce1d4ecd2bf6022fb5fe71871e39373f6e).

**T102: 95% → 96%**. P1 stays **89%** and M0 stays **88%**.
No claim of complete POSIX regex equivalence or physical Hyprland
parity is made; T503 remains 51%, T504 remains 0%, and installed
Mako and the user's real session remain untouched.

## glibc-only POSIX class validation and GNU empty-minimum repetitions

The pinned Mako **v1.11.0** criteria.c compiles summary~ and body~
with glibc regcomp(REG_EXTENDED | REG_NOSUB) in the process's default
C locale. A local C-locale regcomp comparison confirmed that glibc
**rejects** [[:ascii:]], [[:word:]] and the Rust-only per-class
negation [[:^alpha:]] (REG_ECTYPE). Rust's regex accepts those names,
so silently loading these expressions would disagree with Mako's
configuration validation. A scanner in src/config.rs now rejects
those unsupported bracket-class spellings while preserving real glibc
classes such as [[:alpha:]] and [[:digit:]], GNU \w,
and escaped literal opening brackets. The guard does not conflate a
**whole-class** negative [^[:alpha:]] with Rust's unsupported
**named-class** negation.

The same C-locale comparison confirmed that glibc accepts the GNU
repetition spellings a{,2} and a{,} as a{0,2} and a{0,},
respectively. Rust regex parsing normally requires the minimum.
Posthorn normalizes only unescaped {,digits} / {,} outside bracket
expressions and preserves \{,2} literals, so it accepts the
source-compatible syntax without accidentally changing literal text.

Unit tests cover all three invalid class families, valid POSIX/GNU
patterns, ASCII and non-ASCII matches, bounded and unbounded
omitted-minimum repetitions, and escaped braces. Private D-Bus
integration tests reject each invalid class through Reload while
verifying that the **last accepted configuration stays active**.
They then successfully reload a valid [[:alpha:]] rule and verify
ASCII/non-ASCII Notify hook selection; another private service
delivers 0, 1, 2 and 3 a characters through the GNU {,2} matcher
and checks the resulting dismissals.

Both jobs of [CI 40129325687](https://github.com/sguzman/posthorn/actions/runs/40129325687)
passed on [commit ea6730b](https://github.com/sguzman/posthorn/commit/ea6730bc9a6d0e747bda1bc9f48fd7b332e4f400).
The Rust and private D-Bus suites include explicit accepted lines for
glibc_rejects_rust_only_ascii_and_word_posix_bracket_classes,
glibc_gnu_repetition_accepts_omitted_minimum_outside_brackets,
private_reload_rejects_glibc_unsupported_posix_classes_without_style_loss,
and gnu_empty_min_repetition_matches_live_private_notification_criteria.
All headless Sway graphics/scaling/input regressions remain green.

**T102: 96% → 97%**. P1 remains **89%**, and M0 remains **88%**.
Other POSIX ERE edge cases and real Hyprland acceptance remain
unfinished. T503 remains 51%; T504 stays at 0% with **no authorized
production cutover**. No installed Mako instance, session bus or user
configuration was modified.

## Mako CLI option precedence and persistent private-service reload

Pinned Mako v1.11.0 config.c:parse_config_arguments performs TWO
getopt passes: it first loads the selected -c/--config file, then
applies supported global options in original command-line order.
reload_config repeats that operation on each configuration reload
without rearming existing notification timers.

Posthorn now exposes the upstream Mako long-option set through
--check-config PATH, including --key=value and --key value forms,
repeated options, and file selection via -c/--config. Global
overrides apply after reading the file, while matching section
criteria retain priority over the root. Invalid options, missing
values, numeric invalidity and multiline numeric argument
injection are rejected. Validation never launches a daemon.

SharedRuntime::from_path_with_cli_overrides records the original
arguments and replays them on Reload AFTER parsing new disk content,
but BEFORE swapping the accepted configuration. Last-good style
and revision survive invalid source; existing notification
deadlines are not restarted. A private-only service launcher accepts
the same options through --experimental-private-service PATH,
followed by supported Mako long options. The previous duplicate
fallback launcher was removed. The only bus name it may acquire
is org.posthorn.Development, not org.freedesktop.Notifications.

Acceptance covers Config option precedence and schema, an isolated
process invoking the actual posthorn executable, and an independent
private D-Bus client. The process test verifies --default-timeout 75
and --max-history 0, reloads contradictory source defaults, then
verifies both overrides remain effective without a restart. It also
checks standard-bus nonownership and automatically terminates
the fixture process. Separate tests verify rollback after invalid
Reload and that inappropriate experimental-mode arguments are
not silently discarded.

[GitHub Actions run 40131320279](https://github.com/sguzman/posthorn/actions/runs/40131320279)
passed BOTH test and headless-wayland on
[implementation commit 9dcdfad](https://github.com/sguzman/posthorn/commit/9dcdfaddb4e9d00f37d37ad401b3439f4dd7c912).
Its CI log explicitly passes the Rust CLI/Reload tests,
check_config_accepts_mako_cli_overrides_without_starting_a_daemon,
private_dbus_reload_replays_cli_precedence_without_restarting_service
and private_cli_process_uses_cli_timeout_and_reloads_without_public_bus_ownership.
The Sway 1x, fractional scale, 2x, multi-output, pointer and styling
acceptance suite remained green.

**T303: 78% → 82%**; P3 rises **91% → 93%**. Strict M0
stays **88%**. This is tested PRIVATE service behavior,
NOT normal/default production startup, live Hyprland validation or
authorization for cutover. T503 remains 51%; T504 remains 0%.

## Explicit upstream Mako v1.11 root-style initialization

The pinned upstream config.c:init_default_style initializes EVERY root
style member and explicitly marks all style.spec fields specified.
Posthorn previously retained defaults for the visible primary colors
and dimensions but omitted many other initialized properties from the
actual root Options, relying instead on independent fallback values
scattered across the renderer, icon service and runtime.

[Implementation commit 7b4877b](https://github.com/sguzman/posthorn/commit/7b4877bd7633e40d3efbc83b0b12b8a106d7531d)
adds the missing explicit root defaults, including outer-margin 0,
border-radius 0, progress-color #5588AAFF, icons enabled,
max-icon-size 64, icon-path empty, icon-border-radius 0,
icon-location left, text-alignment left, group-by none,
invisible false, output empty, and on-notify none. The earlier
default values, ordered user criteria and root padding state remain
unchanged. This reproduces the upstream root style initialization
rather than merely making its renderer fallback behavior appear close.

The new config unit test verifies all upstream root fields, validates
their option values and checks the resolved ordinary-notification
style. A separate private D-Bus regression sends a real Notify,
checks the live style projection and GetCapabilities, then rasterizes
its Pango/Cairo frame. The test correctly uses the Mako height value
as a MAXIMUM: the short test message measures a smaller actual frame.
Another legacy test expectation was corrected because Mako's
default on-notify binding is explicitly none, not an absent map key.

[GitHub Actions run 40132626637](https://github.com/sguzman/posthorn/actions/runs/40132626637)
passed BOTH full jobs on
[commit db54f24](https://github.com/sguzman/posthorn/commit/db54f24ad97acd353ad6831073b32077adb71658).
The Rust all-targets suite reported 290 passing unit tests; the
private D-Bus suite reported 71 passing tests, including the new
root-style Pango render regression; and Sway 1x, fractional,
2x, multi-output, click-through, pointer and layout regressions
remained green.

**T105: 84% → 86%; P1: 89% → 90%.** The task-weighted strict
**M0 increases from 88% to 89%**, as calculated by progress.json. Unfinished style/layout parity work remains separate:
this evidence does not constitute full physical Hyprland acceptance
(T503: 51%) or an authorized live daemon cutover (T504: 0%).
No installed Mako process, user's home configuration or
production notification bus was touched.

## GNU getopt_long attached -c path parity for private Mako configuration

Pinned Mako v1.11.0 config.c:parse_config_arguments invokes
getopt_long with the h and c: short-option grammar. GNU getopt
therefore accepts an attached required argument as -cPATH in
addition to -c PATH, --config PATH and --config=PATH. Posthorn's
private-service and safe config-check option parser previously
rejected that short-attached spelling.

[Implementation commit 85080b5](https://github.com/sguzman/posthorn/commit/85080b544a6dd9823698f1f2ac1282f02e1c5f2f)
adds the attached form without changing root style precedence or
permitting ordinary production startup. A child-process
check-config test verifies that later -cPATH overrides an earlier
selected file while retaining original option order. A separate
private dbus-run-session fixture uses -cPATH to launch the ACTUAL
Posthorn binary; it verifies timeout=75, max-history=0 and
post-Reload persistence on the disposable development bus. Neither
test uses or replaces org.freedesktop.Notifications.

[CI run 40133198948](https://github.com/sguzman/posthorn/actions/runs/40133198948)
passed test and headless-wayland on
[commit 8f94517](https://github.com/sguzman/posthorn/commit/8f94517741b8e7e144e099a02ae5a76b0033f56b).
The all-targets Rust suite (290 tests), private D-Bus integration
(71 tests), safe CLI integration tests and compositor 1x, 1.25x,
2x, output and pointer suites all remained green.

**T303: 82% → 83%.** P3 remains **93%**, and the strict M0
aggregate remains **89%**. Full Mako startup parity and physical
Hyprland acceptance are still incomplete. T503 remains 51%;
T504 remains 0%. No live daemon, notification bus or personal
configuration was modified.

## GNU getopt_long abbreviated-option parity on isolated CLI

Pinned Mako v1.11.0 config.c:parse_config_arguments uses GNU
getopt_long with its ordered long_options table. glibc accepts
abbreviated long flags such as --wid=480 and --hist=0. A compiled
glibc getopt_long C probe confirmed another less-obvious case:
when multiple Mako style options have the same flag/val and argument
shape, a colliding abbreviation selects the FIRST compatible
declaration, not necessarily an ambiguity error. For example,
--ico=0 resolves to icons (rather than icon-location or icon-path).
An abbreviation colliding with --help and a style option that
have DIFFERENT argument and val descriptors remains ambiguous.

The Posthorn --check-config and --experimental-private-service
parsers now mirror this glibc matching order, retaining
full-name preference, validation of required argument values,
--config selection, safety isolation, and rejection of unknown
or truly ambiguous flags. The behavior is deliberately NOT
enabled for the normal production bus/desktop startup path.
No new executable binary or post-MVP scope was added.

[Implementation commit 0880cd4](https://github.com/sguzman/posthorn/commit/0880cd490f997c4fb368e7b96f7a62ecb26bba37)
adds the ordered prefix matcher. Config-check subprocess tests
verify --conf, --wid, --heig, --hist, --ico, and --mar,
verify that --max=0 rejects using max-icon-size's minimum, and
reject --he=100/--h=1 when help conflicts with style arguments.
The private process-level D-Bus acceptance fixture was also
updated to launch the real executable using abbreviated
--default-t 75 and --max-h 0; it verifies notification expiry,
history suppression, persistent overrides on Reload and that
org.freedesktop.Notifications never becomes owned.

[GitHub Actions run 40134167830](https://github.com/sguzman/posthorn/actions/runs/40134167830)
passed BOTH test and headless-wayland on
[implementation commit 1399c02](https://github.com/sguzman/posthorn/commit/1399c0236997945b7740269289e314980b93403c).
The test job passed all Rust unit tests, private D-Bus integration,
process CLI tests, synthetic QA safety checks and roadmap
consistency. Sway's 1x, 1.25x, 2x, multi-output, input regions,
mixed-DPI, pointer and style acceptance remained green.

**T303: 83% → 84%.** P3 remains **93%**, and the strict M0
aggregate remains **89%**. This closes a documented CLI parity
gap, NOT Mako full startup parity or user workstation QA.
T503 stays at 51%; opt-in T504 stays at 0%. The installed
Mako, real bus owner and user's live session remain untouched.

## C-locale POSIX collating/equivalence class matching

Pinned Mako v1.11.0 criteria.c uses glibc
regcomp(REG_EXTENDED|REG_NOSUB) under the default C locale.
Independently compiled C-locale regcomp/regexec probes verified
that [[.a.]] and [[=a=]] match a single 'a' byte, NOT the
literal dot/equal delimiters; that [a[.b.]c] matches only
a, b and c; that [[.a.]-[.z.]] expresses an ordinary range;
and that multi-character collating elements such as [[.ch.]],
[[=ab=]], and named [[.newline.]] are invalid (REG_ECOLLATE).
They also verified correct parsing of literal ], backslash and
other punctuation inside such elements.

Rust regex interprets some of the syntax as nested sets instead
of POSIX collation. src/config.rs now lowers *only* syntactically
nested C-locale single-byte collating/equivalence elements,
escapes literal punctuation with Rust character-class rules,
keeps valid POSIX [:alpha:] named classes intact, and rejects
multi-character/UTF-8 elements that source glibc cannot compile.
No unsafe C FFI or new runtime dependency was introduced.

Unit tests cover matching a versus dot/equal, ranges, embedded
elements, named classes, literal punctuation and invalid
collation spellings. A private D-Bus test loads the rule through
a file, exercises actual Notify dismissal/retention, rejects an
invalid multi-character element on Reload with last-good-state
and revision preservation, then reloads the equivalent [=a=]
form and repeats the notification checks.

[GitHub Actions 40135401347](https://github.com/sguzman/posthorn/actions/runs/40135401347)
passed BOTH test and headless-wayland on
[implementation commit 0adb46a](https://github.com/sguzman/posthorn/commit/0adb46a92f8fe14209921797b95200d92fd3bd6a).
Sway 1x, 1.25x, 2x, output, input-region, pointer, padding and
layout acceptance remained green.

**T102: 97% -> 98%.** P1 remains **90%**, strict M0
remains **89%**; remaining POSIX ERE discrepancies are NOT
assumed fixed by this narrow source-backed change. Physical
Hyprland acceptance T503 remains 51%; authorized T504
remains 0%. No live Mako, session bus or user config changed.

## glibc GNU word/buffer boundary regex parity

Mako v1.11.0 criteria.c calls glibc
regcomp(REG_EXTENDED|REG_NOSUB). Independently compiled C-locale
glibc probes establish that escaped < and > (backslash plus angle)
are zero-width start/end-of-word assertions; escaped backtick and
escaped apostrophe are zero-width start/end-of-input assertions.
These become literal punctuation when escaped inside a POSIX
bracket class. Other escaped ASCII letters, including A and z,
remain LITERAL under glibc; Rust syntax for those letters is not
accepted directly from the Mako config.

src/config.rs lowers source GNU word assertions to Rust
\b{start}/\b{end}, and source input-buffer assertions to
Rust \A/\z in ASCII (-u) byte-regex mode. The positional behavior,
non-Unicode word classes, bracket punctuation and doubled literal
backslash are covered by unit regressions. The lowerer only runs
on summary~/body~ regex values; normal strings remain unchanged.

The private D-Bus test loads an actual Mako-style config file with
double backslash escaping, because upstream parse_criteria consumes
one escape level BEFORE compiling the regex. It verifies Notify
selection for whole words, substring exclusions, literal angle
punctuation, whole-input assertions and literal backtick/apostrophe
classes, reloading the same private service between configurations
and checking revision numbers. No production service/bus accessed.

Acceptance run:
[GitHub Actions 40139535424](https://github.com/sguzman/posthorn/actions/runs/40139535424)
passed BOTH test and headless-wayland on
[implementation commit 183bed3](https://github.com/sguzman/posthorn/commit/183bed3e416da395b432aa2ba748b96340a55d15).
The test job passed 293 Rust unit tests and 73 private D-Bus
tests; the 1x, 1.25x, 2x, multi-output, pointer/input-region and
layout compositor checks all passed.

Earlier intermediate runs 40139248594, 40139354427, and
40139453718 were RED because synthetic tests initially lacked
the second backslash layer and then contained an unexpanded test
placeholder. Both errors were identified from logs, corrected
in commits 72197e3 and 183bed3, and verified by the final
fully green acceptance run. No failure was treated as success.

**Task T102 98% -> 99% (delta +1 percentage point).**
P1 remains 90% (delta 0pp), strict M0 remains 89%
(delta 0pp). A 99% task is NOT evidence of universal POSIX ERE
equivalence. Physical Hyprland test gate remains T503 51%,
and T504 production switch remains 0%. No live Mako,
personal configuration, session bus or compositor was modified.

## Uncapped compositor integer output scale parity

Pinned Mako v1.11.0 `wayland.c:output_handle_scale` stores the
compositor-supplied `wl_output.scale` integer directly. It does not
clamp the reported output scale to 8. The previous Posthorn
`src/wayland.rs` handler silently clamped to 8, while the frame
scaler, `src/render.rs` and `src/desktop.rs` rejected a valid 9x
scale even for an otherwise small notification.

[Implementation e3028ce](https://github.com/sguzman/posthorn/commit/e3028ce9810165aa71930f89a9be015ef9e0d638)
now preserves all positive compositor scales. `scale_frame`,
native Pango/Cairo painting, and desktop snapshots accept values
above 8 while retaining checked multiplication and the existing
physical-pixel/SHM allocation limits. Reported nonpositive scales
are defensively treated as 1 by the output listener; public frame
helpers reject zero. The fix does not grow safe frame allocations
or relax input-region limits.

Unit regressions cover physical scale 9 with exact pixel replication
and native text raster dimensions; zero and oversized/overflow
scales remain errors. An existing Pango test still asserted that
scale 9 must fail, producing a RED first test job
[40140732219](https://github.com/sguzman/posthorn/actions/runs/40140732219).
[Follow-up test correction c2bc339](https://github.com/sguzman/posthorn/commit/c2bc3391111d616655e88055b40a1bdb7b7afc8b)
replaced that obsolete assertion with the new expected dimensions.

[GitHub Actions acceptance 40140846830](https://github.com/sguzman/posthorn/actions/runs/40140846830)
passed **both** the Rust/private D-Bus and private headless Sway
jobs at `c2bc339`. The Rust job includes 294 library tests
and 73 private D-Bus tests. Headless compositor checks include
1x, 1.25x, 2x, multi-output, mixed-DPI, pointer routing, input
regions and redraw acceptance. **At this earlier checkpoint, no compositor scale-9 screenshot
had yet been accepted**; only the explicit scale-9 Rust pixel and
Pango regressions had run. Physical Hyprland and production Mako
cutover remain untested/unauthorized.

**T401 remains 99% (delta 0pp); P4 remains 99% (delta 0pp);
strict M0 remains 89% (delta 0pp).** This is real, source-backed
correctness progress without inflating acceptance percentages.
T503 stays at 51% and T504 at 0%. No production daemon, bus
name or workstation configuration was touched.

## Actual 9x Sway compositor and pointer acceptance

The source-matched scale fix above is now verified against an actual
isolated Wayland compositor, not only in Rust unit tests. The CI-guarded
`POSTHORN_TEST_SCALE=9` path uses Sway on a dedicated disposable
headless backend at 1920×1080 *physical* pixels (about 213×120
logical). It explicitly confirms the compositor's `get_outputs`
scale of 9, launches Posthorn's private `org.posthorn.Development`
D-Bus service, and paints a small 64-pixel logical-width popup. The
Rust GDK-Pixbuf screenshot probe requires hundreds of new exact-color
pixels relative to a pre-notification compositor screenshot. It then
requires the popup's pixels to return to baseline.

[First full compositor-scale acceptance run 40141755728](https://github.com/sguzman/posthorn/actions/runs/40141755728)
passed both Rust/private D-Bus and Sway CI on
[commit 6648293](https://github.com/sguzman/posthorn/commit/664829357fe9e76d7bd93dba80b8cc4640e62c7d).
That verified real scale 9, pixel presentation and explicit private
D-Bus dismissal. The initial test commit required a Bash escape
correction; only the corrected green run is accepted.

The acceptance was extended to real high-DPI compositor *input*:
`qa_png_probe bounds` determines the painted physical color rectangle;
the midpoint is converted to logical Wayland coordinates (dividing by
9); the existing strictly headless-guarded Rust virtual pointer sends
BTN_RIGHT inside that popup. The fixture then requires both the
screenshot pixels to disappear and the active private Mako-control
list to exclude the dismissed notification. There is **no direct
CloseNotification fallback** in the pointer test, so a broken scale-9
input region would fail the acceptance gate.

[Final acceptance run 40142024109](https://github.com/sguzman/posthorn/actions/runs/40142024109)
passed **both** full Rust/private D-Bus and headless compositor jobs
on [commit 5d7a5f2](https://github.com/sguzman/posthorn/commit/5d7a5f23bd0199f16b29fbffba55c924b9e41cec).
The established 1×, 1.25×, 2×, mixed-output and other compositor
acceptance steps also remained green.

**T403 advanced 98% -> 99% (+1 percentage point)** for successful
high-DPI pointer hitbox and dismissal evidence; T401 stays 99%,
P4 stays 99%, and strict M0 stays 89% (both delta 0pp by the
documented atomic-mean rounding). Physical Hyprland pointer/touch,
actual user monitors, and XDG activation remain separate unclosed
acceptance; T503 is still 51% and T504 is still 0%. The original
workstation daemon, notification bus, desktop config, and input
session were never accessed.

## Non-UTF-8 comment bytes in Mako configuration includes

Pinned Mako v1.11.0 `config.c:load_config_file` uses `getline` and
skips lines whose first non-space/tab byte is `#` before parsing
options or selectors. It never requires a whole config file to decode
as UTF-8. Posthorn's previous `std::fs::read_to_string` failed the
entire config when a harmless comment contained a byte from a legacy
encoding such as 0xFF. This also blocked otherwise-valid nested
includes and private D-Bus Reload.

[Implementation 1601d12](https://github.com/sguzman/posthorn/commit/1601d12644fc9fc7f1bffc46a411f01d471cb4f2)
now reads file bytes and discards comment payloads before decoding
active content. Each discarded comment retains its newline, preserving
the original line locations of criteria sections and parse errors.
Active options remain strict UTF-8; Posthorn does **not** claim exact
C-byte handling for non-UTF-8 option values, because its Rust style
and D-Bus text types require valid UTF-8. Such active lines fail with
a precise file:line diagnostic, never silent lossy replacement.

Unit regressions validate an invalid-byte root comment, an invalid-byte
nested-include comment, source line numbering, resulting style, and
failure on invalid-byte active options. A separate full private service
regression in [commit f4fe843](https://github.com/sguzman/posthorn/commit/f4fe843815680c9ad2ea57ff7935f41cfd713398)
exercises D-Bus Reload through 20 nested include levels with a
non-UTF-8 comment. The live configuration updates, revision increases,
and subsequent invalid values/cyclic includes still preserve the last
known-good revision.

[GitHub Actions run 40144009482](https://github.com/sguzman/posthorn/actions/runs/40144009482)
passed **both** mandatory test and headless-wayland jobs on
`f4fe843`, including Rust unit tests, private service D-Bus
integration and isolated 1x/1.25x/2x/9x compositor acceptance.
No private workstation service, physical Hyprland environment, bus
name, config or input device was accessed.

**T104 87% -> 88% (delta +1 percentage point).**
P1 remains 90% (delta 0pp), strict M0 remains 89%
(delta 0pp). T303 reload work is better regression-covered but
remains 84% (delta 0pp); T503 physical Hyprland remains 51%
and T504 production cutover remains 0%. This does not
establish full byte-oriented Mako config parity.

## Equal-deadline Mako timer selection and expiry order

Mako v1.11.0 `event-loop.c:update_event_loop_timer` selects the
earliest deadline using the **strict**
`timespec_less(&timer->at, &loop->next_timer->at)` comparison.
It *retains* the previously selected timer when a newly added
timer has the same absolute deadline. `add_event_loop_timer`
prepends new entries to the `wl_list`, so only after the chosen
timer is removed does the remaining scan pick the most recently
armed timer among those with the same deadline. `destroy_timer`
invalidates the selected pointer on removal. Sorting tied
deadlines by notification ID (original Posthorn) is not equivalent.

Posthorn's `src/timers.rs` now keeps a selected timer ID as well
as a bounded-by-process-life monotonic `u128` arm sequence.
Insertion with a strictly earlier deadline preempts; an equal
deadline does not. Cancelling the selected timer or expiring it
reselects the newest of the remaining minimum-deadline timers.
Rearming an existing notification destroys the old timer before
creating a new one. The scheduler retains the existing cancellation,
expiry and deadline APIs, with the same deterministic behavior
independently of `HashMap` iteration order.

Unit tests cover equal-deadline selection (first armed, then newest
remaining), ID independence, cancellation, rearming, earlier-deadline
preemption, and deadline rollover through notification history. A
`SharedRuntime` integration regression establishes the *visible*
ordering of expired notification IDs and history storage, not just
the scheduler's internal ordering.

Intermediate [run 40146120591](https://github.com/sguzman/posthorn/actions/runs/40146120591)
was GREEN but tested an overly simple newest-first model that
failed to account for retention of an already selected timer.
Subsequent source inspection exposed the discrepancy and the
intermediate [runs 40146199861](https://github.com/sguzman/posthorn/actions/runs/40146199861)
and [40146207945](https://github.com/sguzman/posthorn/actions/runs/40146207945)
were RED in Rust tests while expectations were being corrected.
Neither is accepted as evidence of correctness.

[Corrected scheduler commit 4fb0bb1](https://github.com/sguzman/posthorn/commit/4fb0bb162c58c0c5310e50f8286bf1f1260bfb49)
passed both mandatory CI jobs in
[run 40146219104](https://github.com/sguzman/posthorn/actions/runs/40146219104).
[Runtime integration commit 9e58623](https://github.com/sguzman/posthorn/commit/9e5862301add3784a4f4c67de5bb7b33222739c4)
passed both mandatory CI jobs in
[run 40146392300](https://github.com/sguzman/posthorn/actions/runs/40146392300):
300 Rust library tests and 73 private D-Bus integration tests passed,
as did the existing isolated Sway 1x, 1.25x, 2x and 9x compositor,
multi-output, mixed-DPI, input region and redraw acceptance steps.

**T203 98% -> 99% (+1 percentage point); P2 96% -> 97%
(+1 percentage point); strict M0 remains 89% (0pp).**
This is source-backed timer parity, not a claim that every possible
timer/event-loop interleaving has been accepted: the Rust
`take_expired` operation gathers due expirations in one pass,
whereas Mako invokes each timer callback between selections.
Physical Hyprland T503 remains 51%, and T504 remains 0%.
No production bus, running Mako, workstation input, or personal
configuration was accessed.
