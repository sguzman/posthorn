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

## Output selectors require compositor-established surface identity

Pinned Mako v1.11.0 `criteria.c:match_criteria` explicitly rejects
an `[anchor=...]` selector when `notif->surface == NULL`, and an
`[output=...]` selector when either `notif->surface == NULL`
**or** `notif->surface->surface_output == NULL`. Mako
`dbus/xdg.c:handle_notify` applies the initial criteria before
associating a new notification's layer surface, while
`wl_surface.enter` establishes its actual output later.
A configured target output is not a compositor output observation.

Posthorn previously encoded both missing fields as empty Rust strings.
That allowed the syntactically valid but ordinarily unmatchable
`[output=]` selector to match a fresh notification before it even
had a surface, affecting default-timeout and on-notify behavior.
`src/config.rs::Traits` now records **separately** whether a
surface is associated and whether its compositor-enter output is
known. Anchor/output criteria reject missing associations even if
their value is the empty string. The distinction propagates through
runtime scheduling, visible notification styling and the synthetic
hidden-notification projection.

Surface reuse also uncovered a secondary mismatch: the legacy
single-surface compositor output is represented by an unset
per-notification override (`None`), not an explicit absent observation
(`Some(None)`). Posthorn no longer manufactures `Some(None)`
when a notification is rebound to a known surface lacking an
individual observation. This preserves actual output matching
through direct-ID replacements without conflating unset, unknown
and known compositor output names.

[Source/fixture implementation 35b6239](https://github.com/sguzman/posthorn/commit/35b6239ea4e278a3e39492b6ffec9c6804a0935f),
[private D-Bus regression 24688b3](https://github.com/sguzman/posthorn/commit/24688b30c7fc0329c967c78a4bcbdbc8e5cb8a49),
[surface reuse correction 0d64846](https://github.com/sguzman/posthorn/commit/0d64846f805e6d6392ceb12240f7da939128d1a7)
and [full fixture correction 5270a36](https://github.com/sguzman/posthorn/commit/5270a3623a97375beb0ec2a21e971fd0ed2caa76)
are committed on `main`. New tests verify the surface/enter
preconditions directly, the absence of a spurious timeout on initial
Notify, an existing surface with no entered output, the positive case
of a genuinely entered *empty-name* output on replacement, and
rejection when the same surface subsequently enters `HEADLESS-1`.

Intermediary runs
[40148329066](https://github.com/sguzman/posthorn/actions/runs/40148329066),
[40148398154](https://github.com/sguzman/posthorn/actions/runs/40148398154)
and [40148409774](https://github.com/sguzman/posthorn/actions/runs/40148409774)
were red while all Rust `Traits` constructor call sites were being
updated. They are not acceptance evidence. The corrected final
implementation passed both mandatory jobs in
[run 40148493499](https://github.com/sguzman/posthorn/actions/runs/40148493499).
Its test job passed 302 library tests, 74 private D-Bus integration
tests, and remaining targeted tests, including the new private
surface lifecycle case. The isolated Sway job passed the 1x,
fractional 1.25x, 2x, 9x and multi-output / input / DPI QA suite.
No real Hyprland session or production notification service was used.

**T105 86% -> 87% (+1 percentage point); P1 90% -> 91%
(+1 percentage point); strict M0 remains 89% (0pp).** Neither
exact physical workstation behavior nor every cross-output
criteria/render interaction is asserted complete. T503 physical
Hyprland stays at 51%, T504 cutover at 0%.

## Hidden overflow placeholder allocation advances Mako notification IDs

Pinned Mako v1.11.0 `render.c:render_surface` (lines 442-479)
calls `create_notification(state)` whenever a surface has
`hidden_count > 0`. It sets `hidden=true`, applies the hidden
style and **only then** checks whether the pseudo-notification is
`invisible`. It ultimately calls `destroy_notification` even
when nothing is painted. `notification.c:create_notification`
increments `state->last_id` at allocation (line 80). Consequently,
every such redraw consumes an ID, observable as skipped IDs in
subsequent sender-facing Freedesktop `Notify` responses.
No pseudo-notification enters the active notification list or
history, and no D-Bus `NotificationClosed` signal is involved.

Posthorn previously rendered the hidden placeholder without
consuming a notification ID. The renderer now calls
`NotificationStore::reserve_hidden_render_id` once per overflow
render, before testing the placeholder's `invisible` style. It
reserves through the existing `allocate_id` path but does not
register an active instance, store a payload, change history or
emit notification signals. Pure visibility projections and input
hit-testing do not reserve IDs. This is specifically a rendering
side effect, not a side effect of merely *counting* overflow.

[Renderer fix c8a5b20](https://github.com/sguzman/posthorn/commit/c8a5b20c6351fe6e03b2cbed4f6e0a356012425a)
adds regressions for two consecutive overflowing redraws,
last-good active list preservation, no allocation when overflow
ends, and allocation even under `[hidden] invisible=1`.
[Private D-Bus regression 6fa52d8](https://github.com/sguzman/posthorn/commit/6fa52d849f8ba2a763bd369a895d91cfba1b3153)
sends two real `Notify` requests with IDs 1 and 2, performs
two overflowing raster snapshots, verifies that the next
`Notify` returns 5, and checks that `ListNotifications`
contains only the three sender-created records.

[GitHub Actions run 40153207109](https://github.com/sguzman/posthorn/actions/runs/40153207109)
completed with both mandatory jobs **green** on `6fa52d8`.
304 Rust library tests, 75 isolated private D-Bus integration
tests, and the 1x / fractional / HiDPI / 9x / multi-output /
input-routing Sway acceptance steps all passed. The live user's
session bus, Mako instance, Hyprland environment and configuration
were never accessed.

**Desktop notification parity feature: 98% -> 99% (+1pp).**
The T201 atomic ID task was already recorded as 100%, while T204
hidden layout remains 99%. No further atomic-task completion is
claimed from this one fix; P2 remains 97% (0pp) and strict M0
remains 89% (0pp). Exact hardware/event-loop scheduling,
physical Hyprland T503 (51%) and T504 cutover (0%) remain open.

## Percent-decoded NUL in Mako file-URI icon names

Pinned Mako v1.11.0 `icon.c:url_decode` copies each correctly
encoded `%xx` byte into a conventional NUL-terminated C filename.
`resolve_icon` passes that filename to `load_image`, which uses
`strlen(path)` and `gdk_pixbuf_new_from_file`. A `%00`
therefore terminates the filename **at that byte**; later apparent
URI characters are ignored. If it is the first byte, `load_image`
rejects the resulting empty path. An ordinary raw `%nothex`
sequence is still copied literally, and non-UTF-8 decoded bytes
remain legal Linux filename bytes.

Posthorn's earlier `src/icons.rs:url_decode` rejected the entire
path on `%00`. Its Rust decoder now truncates to the preceding
byte prefix, which is passed through a Linux `OsString`, and
returns no resolved icon for an empty resulting filename. This
does not change the already-permitted absolute/relative
`file://` path policy or the strict unprefixed theme-name
validation. The decoder does **not** introduce an embedded NUL
into a Rust `Path`: it stores only the C-observable prefix.

[Decoder commit baa00cc](https://github.com/sguzman/posthorn/commit/baa00ccab2ee1898f05a3381ddb6e43adddc3c45)
and [GdkPixbuf test f999676](https://github.com/sguzman/posthorn/commit/f999676da77eb8ab3f179ef27c4cd10a0d715613)
cover `file:///tmp/example.png%00ignored`,
the empty-result case, non-UTF-8 suffix avoidance and a real
temporary PNG read from its truncated file URL.
[Private D-Bus/Cairo regression 5d35870](https://github.com/sguzman/posthorn/commit/5d35870aec87cddfa9f40e31ad992748ccae8502)
sends `image-path=file://<fixture>.png%00not-the-real-filename`
over the isolated service, verifies the unmodified original hint
and detects the correctly loaded PNG's opaque red pixels in the
rendered Cairo frame.

[GitHub Actions run 40154710747](https://github.com/sguzman/posthorn/actions/runs/40154710747)
is **green** for both required jobs at commit `5d35870`:
305 Rust library tests and 76 isolated private D-Bus tests passed,
as did the existing 1x, fractional, HiDPI, 9x, mixed-output and
compositor input acceptance suite. Earlier implementation
revisions are superseded; only the final green run is counted.

**T402 remains 99% (0pp)**; passing a path-decoding test does
not establish full text/icon compositor parity. P4 remains 99%
(0pp), strict M0 remains 89% (0pp), T503 physical Hyprland
acceptance remains 51% and T504 production cutover remains 0%.
No private config, production session bus or installed Mako
was accessed or changed.

## Correction: hidden is NOT a Mako v1.11.0 group-by field

Source re-audit on 2026-10-25 identified a mistake in an
earlier accepted parity claim. In pinned upstream
`types.c:parse_criteria_spec` (lines 181-230),
`hidden` is **absent** from the complete set of recognized
grouping field names; an unknown token returns `false`.
`criteria.c:parse_criteria` separately accepts `[hidden]`
as a **selector**. `criteria.c:validate_criteria` checks
that selected **already parsed** grouping fields are
compatible, but it does not expand which fields may parse.

The earlier change 7aaab59/87a5422 incorrectly accepted
`group-by=hidden` and added a passing D-Bus regression
that tested non-upstream behavior. The corrected implementation
removes `GroupField::Hidden`, rejects `group-by=hidden`
and its combinations in configuration and grouping parsing,
and adds negative regression tests grounded in the pinned
`types.c` parser. Its former positive test is removed.
The green CI run 40155658468 proves the former code compiled
and worked as its incorrect test expected; it is **not**
evidence of Mako compatibility for this behavior. Earlier
source-baseline claims for this field are superseded.

The previously awarded progress point has been withdrawn:
**T103 80% -> 79% (-1pp correction), P1 91% (0pp), M0 89% (0pp).**
No real Mako service, workstation bus or Hyprland session
was used.

## Group-by parser contract reverified with isolated D-Bus Reload

The correction commit
[1544bdc](https://github.com/sguzman/posthorn/commit/1544bdc5d4a770c20b3e40c9994c4fa1631f040d)
has [green CI run 40197080478](https://github.com/sguzman/posthorn/actions/runs/40197080478).
A second [source-contract test commit e2c4f2e](https://github.com/sguzman/posthorn/commit/e2c4f2ec848fa86f4e7a8ab7bdd0f9343333e96c)
exhaustively exercises each valid `types.c:parse_criteria_spec`
field and rejects selector-only fields, including `hidden`
and `mode`. Its private D-Bus Reload regression shows that
`group-by=hidden` and `group-by=none,hidden` raise the
`fr.emersion.Mako.InvalidConfig` method error while the
previous valid `[hidden]` selector configuration remains
active; a later valid `group-by=app-name` reload succeeds.

[CI run 40197573630](https://github.com/sguzman/posthorn/actions/runs/40197573630)
passed both jobs: 308 Rust library tests, 77 isolated private
D-Bus integration tests, plus the full existing headless Sway
compositor acceptance matrix. This strengthens evidence for the
**corrected 79% T103**, not for the rejected false-positive
`group-by=hidden` implementation. Phase P1 is still 91%,
strict MVP 89%.

## Mako icon lifetime, retained fit and initial compositor scale — accepted

Pinned Mako v1.11.0 `dbus/xdg.c:handle_notify` (lines 401-413)
calls `create_icon` **once** after applying the new notification's
style. `icon.c:create_icon` (lines 253-288) caches a decoded
pixel surface with initial `max-icon-size` fit and the theme asset
selected at the maximum currently attached output scale.
`dbus/mako.c:reapply_config` (lines 302-334) changes styles and
reassigns surfaces without recreating the cached icon. A history
entry retains its icon; only a genuine replacement `Notify`
creates a new one. `render.c` uses `notif->icon` if the current
style still enables icons.

Posthorn previously loaded icon files and theme assets again on
every frame. Now `Notification::icon_snapshot` represents
**uncaptured**, **captured but absent** and **captured pixels plus
original fit size**, preventing both retroactive icon changes and
late-appearing files from altering a prior notification.
`SharedRuntime::schedule_impl` captures exactly once at
notification arrival after initial criteria style selection.
Direct-ID and matching stack-tag replacements reconstruct the
notification and capture a fresh icon; mode changes, file reload,
history close and history restore keep the original snapshot.
The size cap is retained during Pango measurement AND Cairo paint.
Direct unscheduled in-memory renderer fixtures still allow
on-demand decoding without pretending they went through Notify.

Mako also knows its outputs before accepting notifications.
Posthorn now performs a registry-only Wayland scale probe before
registering the private D-Bus name, and its multi-surface renderer
keeps an independent output observer even when there are zero
notifications. The evolving maximum monitor scale applies only
to **new** icon captures, never to previous notifications.
A QA-only `POSTHORN_QA_ICON_TRACE` diagnostic prints only
the scale, max-size and a captured pixel; it is not enabled
in normal operation and never logs notification text, icon paths
or user-supplied identifiers.

Source implementation across commits:
[dd5914f](https://github.com/sguzman/posthorn/commit/dd5914fef33dccc4f84853d245f02c4e7c9355c2),
[4b7de16](https://github.com/sguzman/posthorn/commit/4b7de165a6d7d4a3b8651c81e084be655dda4b87),
[7bfc194](https://github.com/sguzman/posthorn/commit/7bfc194ddb7b2aa0dc0afe682972e594bf6a599f),
[d1c3b43](https://github.com/sguzman/posthorn/commit/d1c3b436a3571e5d399fb950c7ee77867583fff2).

Acceptance includes:
- Private D-Bus Notify with a real PNG that changes on disk;
  the old frame stays identical, then direct replacement refreshes
  it. A missing PNG is not retried until a new Notify arrives.
- `SetMode` changes `max-icon-size` but does not resize the
  existing icon, while subsequent Notify captures the new fit.
- Private `Reload`, `CloseNotification` and
  `RestoreNotification` after removal of the PNG retain the
  original pixels and fitted dimensions.
- Unit and private D-Bus tests select 1x versus 2x themed PNGs
  at **arrival** and confirm later lookup/output-scale changes
  cannot change an existing notification's asset.
- Real isolated Sway mixed-DPI QA keeps the older icon on an
  output scale change, then verifies subsequent new notifications
  on 1x and 2x with screenshot color/height checks and real
  virtual-pointer dismissal at the bottom of the icon.

Intermediate CI runs were red while updating compile fixtures and
replacing the **incorrect old test** that expected redraw-time
icon switching. The final source implementation passed both jobs
at [run 40200454039](https://github.com/sguzman/posthorn/actions/runs/40200454039):
309 Rust library tests, 81 isolated private D-Bus integration
tests and the full compositor acceptance suite (1x, fractional
1.25x, 2x, 9x, multi-output, mixed-DPI and input).
No live Mako installation, production D-Bus bus or user
Hyprland configuration was touched.

This closes the identified icon lifetime/source-parity discrepancy,
but **does not establish every icon, SVG/Pango or Wayland behavior
as 100%**. T402 is still 99% (0pp), T401 is still 99% (0pp).
Documented and tested startup/output-scale knowledge plus reload
retention advances **T303 84% -> 86% (+2pp)** and
**P3 93% -> 94% (+1pp)**. Strict M0 remains **89% (0pp)**;
physical Hyprland T503 remains 51% and cutover T504 remains 0%.

## Initial icons=0 and later mode toggles preserve Mako icon lifetime

Pinned Mako v1.11.0 `dbus/xdg.c:handle_notify` only invokes
`create_icon` when **the initial matched style has `icons=1`**.
The cached `notif->icon` is retained across
`dbus/mako.c:reapply_config`; only its visibility is controlled
by the current `style.icons` when `render.c` paints the frame.
Therefore an initially iconless notification cannot suddenly
load an icon if a later `SetMode` enables icons. Conversely an
already cached icon survives `icons=0` and is restored from
the **original decoded bytes** when `icons=1` is reenabled,
even if its source PNG changed in the meantime.

[Regression 1473a9c](https://github.com/sguzman/posthorn/commit/1473a9ca3eff00b85c249c1870ed0e11bee3611f)
sends real Notify requests through the isolated private D-Bus:
(a) SetMode(hide), Notify, SetMode(show) must retain
`icon_snapshot=Some(None)` and the iconless frame;
(b) SetMode(show), Notify, SetMode(hide), modify PNG,
SetMode(show) must paint precisely the cached initial PNG.
[Fixture lifetime correction c5cede1](https://github.com/sguzman/posthorn/commit/c5cede16125176d75f0a8df40705f81a5e6864a7)
makes the test's dynamically selected summary an owned string.

The first interim [run 40201487894](https://github.com/sguzman/posthorn/actions/runs/40201487894)
failed a Rust test-fixture lifetime compilation check; it is not
an acceptance result. [Run 40201668971](https://github.com/sguzman/posthorn/actions/runs/40201668971)
passed **both mandatory jobs** after that correction:
309 Rust library tests, 82 isolated private D-Bus tests, and
the complete headless Sway compositor suite, including mixed DPI,
1x, 1.25x, 2x and 9x, input, redraw and multi-output checks.
Installed Mako, production bus, and the user's live session
were not accessed.

This expands acceptance of the retained icon-lifetime change.
It does not justify adding another point to the
almost-finished T402 rendering task, so
**T402 stays 99% (0pp); T303 stays 86% (0pp); P3 stays 94%
(0pp); strict M0 stays 89% (0pp)** for this incremental
test. The previous, separately verified icon implementation
already advanced T303 from 84% to 86% (+2pp), and P3 from
93% to 94% (+1pp) this development session.

## Themed icon glob candidate parity (2026-10-25)

Pinned Mako v1.11.0 `icon.c:resolve_icon` searches
`%s/*/*/%s.*` using `glob(..., GLOB_NOSORT)`. It does not sort
the returned matches, require regular files or pass `GLOB_PERIOD`.
For each theme, the **first** nominal-size/scale exact match wins.
Otherwise Mako retains the **first** candidate at the greatest
eligible nominal size: `icon_size > last_icon_size` replaces a
fallback, but equal sizes do not. An unmatched dot-prefixed directory
is not traversed by either `*` directory component. Matching
directories and dangling symlinks are still eligible paths, even
though subsequent GdkPixbuf decoding may fail.

Posthorn previously sorted candidates by filename, searched dot-prefixed
wildcard directories, and filtered out nonregular candidates. These
three deviations are now corrected without changing the configured
icon search roots or bypassing GdkPixbuf decode errors.
The new source-grounded Rust regressions cover exact-size ties,
same-size fallback ties, both levels of dot-prefixed directories,
and matching directories/dangling symlinks.

- [First-match selection `9f360d3`](https://github.com/sguzman/posthorn/commit/9f360d3168322ab9a3d9e41c914a3918fb003c71),
  [green CI](https://github.com/sguzman/posthorn/actions/runs/40202864882).
- [Hidden glob directories `e4ff530`](https://github.com/sguzman/posthorn/commit/e4ff5304b3d74c3241dd7e7dc55e2412adac3e83),
  [green CI](https://github.com/sguzman/posthorn/actions/runs/40203038250).
- [Nonregular matches `614cd47`](https://github.com/sguzman/posthorn/commit/614cd4754c0bd143cbd30a87c60fb2cf4454b8a5),
  [final green CI](https://github.com/sguzman/posthorn/actions/runs/40203108396).

Final CI passed **312 Rust library tests**, **82 private D-Bus
integration tests**, and both mandatory CI jobs including the isolated
headless Wayland compositor suite. This acceptance covers the listed
icon lookup cases; it does not assert that GdkPixbuf or filesystem
enumeration behaves identically on all filesystems, that every Mako
rendering behavior is complete, or that physical Hyprland acceptance
has occurred. **T402 remains 99% (delta 0pp), P4 remains 99%
(delta 0pp), strict M0 remains 89% (delta 0pp), physical
Hyprland acceptance remains 51% (delta 0pp), and cutover
remains 0% (delta 0pp).** Live Mako and workstation settings
were not changed.

## Inline image-data channel metadata versus GdkPixbuf pixel layout (2026-10-25)

Pinned Mako v1.11.0 `dbus/xdg.c:handle_notify` uses the sender's
`channels` integer when computing how many bytes of the `image-data`
(`(iiibiiay)`) hint to copy:
`(height - 1) * rowstride + width * ceil(channels * bits_per_sample / 8)`.
Later `icon.c:load_image_data` passes **has_alpha**, not that reported
channel count, into `gdk_pixbuf_new_from_data`. With eight-bit samples,
GdkPixbuf reads three RGB channels when `has_alpha=0` and four RGBA
channels when `has_alpha=1`. Thus Mako can render an otherwise safely
sized notification even when the metadata's declared channel count is
larger than the GdkPixbuf pixel count.

Posthorn previously rejected every mismatched channel count. It now
separately checks the Mako D-Bus wire-copy length and GdkPixbuf's
alpha-selected per-pixel stride. It accepts safely sized oversized
counts, while rejecting counts that would leave the C decoder reading
past copied data. The existing maximum allocation/image dimension
bounds remain in force; **unsafe, undersized or malformed inputs are
not emulated for crash parity**.

The regression exercises both RGBA with reported five channels and
opaque RGB with reported four, plus undersized negative cases. A
real isolated `org.posthorn.Development` Notify test confirms the
received pixel bytes and retained icon snapshot for both variants,
without claiming ownership of the workstation notification bus.

- [Implementation 3d1892b](https://github.com/sguzman/posthorn/commit/3d1892bf7ee3251377f5105c6f0ac0bca44fd4b5)
- [Private D-Bus acceptance 050a151](https://github.com/sguzman/posthorn/commit/050a1511394818acedbe14af627c397808401b1a)
- [Green mandatory CI run 40204356917](https://github.com/sguzman/posthorn/actions/runs/40204356917):
  **313 Rust library tests, 83 private D-Bus tests, full isolated
  headless Wayland compositor suite**.

This accepts the specific wire/pixel-layout discrepancy and advances
**T301 98% → 99% (+1 pp)**. It does not establish full Notify protocol
parity or physical compositor acceptance. P3 remains **94% (0 pp)**,
strict M0 remains **89% (0 pp)**, physical Hyprland acceptance remains
**51% (0 pp)**, and cutover remains **0% (0 pp)**. No live Mako,
Hyprland or production D-Bus configuration was changed.

## Malformed inline image-data: deliberate bounded-wire safety exception (2026-10-25)

Pinned Mako v1.11.0 `dbus/xdg.c:handle_notify` computes an expected
byte count from the `(iiibiiay)` image-data hint before reading each
declared byte using `sd_bus_message_read(msg, "y", &tmp)`. Its loop
checks `ret < 0` but not `ret == 0`; the libsystemd
`sd_bus_message_read_basic` API explicitly returns zero at the end
of the currently opened array. Therefore it is **not source-grounded
to claim Mako reliably rejects an undersized byte array**. The upstream
implementation may instead continue using the uninitialized C
`tmp` variable. We do not emulate that unsafe behavior.

Posthorn safely computes
`(height-1)*rowstride + width*ceil(channels*bits_per_sample/8)`
using checked unsigned arithmetic and rejects a truncated sender array
*before any notification replacement, ID allocation, or alias collapse*.
This validation is separate from GdkPixbuf pixel acceptance: a complete
image array using an unsupported bit depth is accepted by Notify but
retains no rendered icon, and safely oversized channel metadata still
decodes using the `has_alpha`-selected stride (see previous section).

Evidence and limitations:

- [Pinned upstream D-Bus image loop](https://github.com/emersion/mako/blob/v1.11.0/dbus/xdg.c#L258-L335)
- [libsystemd sd_bus_message_read_basic return semantics](https://www.freedesktop.org/software/systemd/man/latest/sd_bus_message_read_basic.html)
- [Wire validation implementation 91a2b44](https://github.com/sguzman/posthorn/commit/91a2b4449fa352d79c9128fc39bedcc9f99d6bbb)
- [Private D-Bus end-to-end regression 6418680](https://github.com/sguzman/posthorn/commit/641868024a622d79f8d9eb8a188b47a733707219)
- [Green mandatory CI 40206069699](https://github.com/sguzman/posthorn/actions/runs/40206069699):
  **314 library tests; 84 private D-Bus tests; isolated headless
  Wayland compositor suite passed**.

This is an **intentional safety hardening, not a strict upstream parity
claim**. It earns **no percentage credit**: T301 stays **99% (+0 pp)**,
P3 **94% (+0 pp)**, M0 **89% (+0 pp)**, physical Hyprland
acceptance **51% (+0 pp)**, and production cutover **0% (+0 pp)**.
Live Mako, the real freedesktop notification bus and workstation
settings are unchanged.

## POSIX ERE bracket classes versus Rust regex set syntax (2026-10-25)

Pinned Mako v1.11.0 `criteria.c:apply_criteria_field` compiles
`summary~` / `body~` using
`regcomp(REG_EXTENDED | REG_NOSUB)`. Under the daemon's initial C
locale this is the glibc POSIX ERE grammar, not Rust `regex::bytes`'s
additional bracket-set algebra. Native glibc `regcomp` / `regexec`
probes confirmed two sender-visible selector differences:

1. `[a&&b]` matches `a`, `&` and `b`; `[a~~b]` matches
   `a`, `~` and `b`. glibc treats both repeated punctuation
   characters as literals in a class, whereas unescaped Rust
   `regex` uses `&&` and `~~` for intersection/symmetric difference.
   Posthorn now escapes literal `&` and `~` within bracket classes
   during the existing POSIX-to-Rust translation. Collating/equivalence
   elements and escaped punctuation still preserve their intended meaning.
2. A closing bracket as the **first** class character, or the first
   character after negating `^`, is itself a POSIX class member.
   For example `[]\\w]` in a C-locale glibc ERE matches `]` or
   the letter `w`, not an arbitrary word character. Likewise
   `[^]\\w]` excludes both `]` and `w`.
   A preprocessing pass now escapes that literal leading bracket
   *before* other POSIX bracket-element and backslash transformations.

Regression cases cover positive and negative glibc results for plain,
negated, repeated-punctuation, named-class-composite, and escaped
variants, plus full `Config::parse` and scoped `summary~` and
`body~` matching. Those fixtures were verified against the pinned
source contract, not inferred from Rust regular-expression behavior.

- [Literal ampersand/tilde patch 2758499](https://github.com/sguzman/posthorn/commit/2758499a640aa18ccb93cc6851c17862df4222ff)
- [Escaped-punctuation fixture 7ee844a](https://github.com/sguzman/posthorn/commit/7ee844a6fa2f2c625909c83fe09b01660719ee58)
- [Literal leading bracket patch 2a17079](https://github.com/sguzman/posthorn/commit/2a17079e585d4d27451d61aeacdc55aba5799048)
- [Green mandatory CI 40208648385](https://github.com/sguzman/posthorn/actions/runs/40208648385):
  **316 library tests, 84 private D-Bus tests, complete isolated
  headless Wayland suite**; no failures.

This accepts two previously divergent POSIX regex behaviors:
**T103 79% → 81% (+2 pp)**. The P1 phase arithmetic mean is now
**91% (+0 pp)** and 24-task strict M0 arithmetic mean **89.083%**,
still reported as **89% (+0 pp)**. Other POSIX grammar differences,
including full glibc bracket range handling/backreferences, are
not accepted here; this does not certify complete regex parity.
Physical Hyprland remains **51% (+0 pp)**, and explicit production
cutover remains **0% (+0 pp)**. The installed Mako and real D-Bus
name were never accessed.

## POSIX ERE bracket member semantics: literal backslashes, pipes, opening bracket (2026-10-25)

Building on the earlier accepted `&&`, `~~` and leading `]`
compatibility fixes, native glibc `regcomp(REG_EXTENDED|REG_NOSUB)`
and `regexec` probes in the initial C locale exposed **three more
configuration-selector divergences** from Rust `regex::bytes`:

1. **A backslash inside a POSIX bracket class is a literal member, not
   an escape for the next character.** For example, Mako's
   `[\\w]` matches both the character `\\` and `w`. The
   class `[a\\-b]` includes the range from `\\` to `b`
   (including `]`, `^`, `_`, `a`, `b`) and does **not**
   match a hyphen. The old translator consumed backslashes before
   letters and interpreted escapes before punctuation as Rust escapes,
   changing both class membership and range boundaries. A new
   bracket-aware prepass retains every C-locale bracket backslash as a
   Rust literal backslash while leaving the GNU `\\w` operator
   **outside** classes untouched.
2. **`||` is literal in glibc bracket classes.** Mako accepts
   `[a||b]` and matches the pipe symbol in addition to `a` and
   `b`. Rust regex reserves `||` for explicit class union.
   Posthorn now escapes literal pipes inside POSIX bracket classes,
   extending its prior handling for `&&` and `~~`.
3. **An unescaped `[` inside a bracket class is an ordinary class
   member unless it begins a POSIX named class, collating element or
   equivalence element.** Mako accepts `[[]`, `[a[]` and
   `[^[]`. Rust regex can interpret the internal `[` as a nested
   character set, so Posthorn now quotes the ordinary literal opening
   bracket while preserving valid `[[:digit:]]` etc.

The regression suite tests glibc-confirmed positive and negative
matches for `[a\\-b]`, `[a\\^b]`, `[a\\&b]`,
`[a\\~b]`, `[\\w]`, `[\\d]`, `[\\-]`,
`[\\^]`, `[a\\-z]`, `[a\\d-z]`,
`[a\\[:digit:]]`, `[a||b]`, `[||]`,
`[[]`, `[a[]`, `[a[b]`, `[^[]`, mixed literal
`[`/named-class contents, and full config `body~`
selectors. These are **actual C-locale matching semantics**, not
just successful compilation of a Rust regex.

- [POSIX bracket backslash implementation 2552f12](https://github.com/sguzman/posthorn/commit/2552f12f24c3ddd293cd3a5d1b7a72b87a9c1b65)
- [POSIX pipe literal implementation bdc3d53](https://github.com/sguzman/posthorn/commit/bdc3d53247f43ea02f27c0844a9fdbc806f37d6e)
- [POSIX opening-bracket implementation fa27579](https://github.com/sguzman/posthorn/commit/fa275791da53344c68a45e26fe73b095b7bdf8dd)
- [Green required CI 40211609315](https://github.com/sguzman/posthorn/actions/runs/40211609315):
  **318 Rust library tests, 84 private D-Bus tests and the full
  isolated headless Wayland compositor suite all passed.**

Earlier notes asserting that bracket-escaped letters were just
unescaped into ordinary letters were incomplete: glibc also preserves
the backslash *as a matching character*, which this source-verified
patch corrects. Mako's external `\\w`, `\\s` and word
boundaries remain separate operations. A complete POSIX ERE matcher
(including all range/collation and backreference edge cases) is **not
claimed**.

Accepted: **T103 81% → 84% (+3 pp)** and P1
**91% → 92% (+1 pp, raw average 91.0% → 91.6%)**.
The 24-task strict M0 average rises from **89.083% → 89.208%**
but displays as **89% (+0 whole-number pp)**.
Physical Hyprland remains **51% (+0 pp)** and production
cutover **0% (+0 pp)**. Real Mako, Wayland compositor and
notification bus were not accessed.

## Mako notification closure: history-gated Notifications property invalidation (2026-10-25)

Pinned Mako v1.11.0 `notification.c:close_notification` invokes
`notify_notification_closed(notif, reason)` **before** checking
whether the active notification's resolved `style.history` is disabled
or `config.max_history <= 0`. When either is disabled, it calls
`destroy_notification` and **returns early**, bypassing
`emit_notifications_changed(state)` at the bottom of that function.
That last call emits `PropertiesChanged` invalidating the Mako control
interface's `Notifications` property. The ordinary freedesktop
`NotificationClosed` signal nevertheless **always fires** for the
actual close. By contrast, an explicit `dismiss --no-history` with
`history=1` and positive `max-history` still reaches the invalidation
path: `add_to_history=false` suppresses archiving, not the signal.

Posthorn previously invalidated `Notifications` unconditionally on
every closed notification, including `history=0` and
`max-history=0`. It now reports both the closure and the
source-dependent invalidation decision, with the same policy applied
consistently to:

- the freedesktop `CloseNotification` D-Bus method;
- Mako control `DismissNotifications` (single, group, all,
  and `history=false`);
- touch/pointer/action and `on-notify` binding dismissal; and
- timer-driven `NotificationClosed` expiration.

The runtime keeps existing backward-compatible `close()` and
`expire()` APIs; new `close_with_change()` and
`expire_with_changes()` expose the D-Bus invalidation decision
without allowing D-Bus signal emission inside a synchronous
notification-store lock. No signals are emitted for a second close
of an inactive ID. The change does **not** suppress ordinary
`NotificationClosed` or change history eligibility, grouping,
timeouts, or visible raster output.

Acceptance includes unit tests for `history=0`,
`max-history=0`, regular history-enabled close, explicit
no-history dismissal, positive and negative expiration cases,
and a real **private** D-Bus signal subscription proving:

1. `Notify` emits the normal initial `Notifications` invalidation.
2. `history=0` `CloseNotification` emits `NotificationClosed`
   with reason 3 but no second `Notifications` invalidation.
3. A category-scoped `history=1` close emits both.
4. A control dismissal using `history=false` still invalidates but
   does not archive that notification.

- [Pinned source: notification.c close_notification](https://github.com/emersion/mako/blob/v1.11.0/notification.c#L109-L147)
- [Pinned source: control property invalidation](https://github.com/emersion/mako/blob/v1.11.0/dbus/mako.c#L479-L495)
- [Runtime implementation acd1d16](https://github.com/sguzman/posthorn/commit/acd1d166b9f0a450bc78e27de12b6ff120a63249)
- [Private D-Bus regression c82e4cb](https://github.com/sguzman/posthorn/commit/c82e4cb5d55539833f6d2b283c2ff5355cb6c362)
- [Private max-history=0 expiry regression c181fba](https://github.com/sguzman/posthorn/commit/c181fbaafe0a0d8913adefb9845fa8a27722dda9)
  confirms the other early-return condition over real isolated D-Bus:
  reason=1 `NotificationClosed`, no Notifications invalidation,
  and no archived entry.
- [Complete accepted CI 40214134406](https://github.com/sguzman/posthorn/actions/runs/40214134406):
  **320 Rust library tests**, **86 isolated private D-Bus tests**,
  and **full isolated headless Wayland acceptance**, all passing.

Accepted progress: **T302 98% → 99% (+1 pp)**,
**P3 94% → 95% (+1 pp)**. The 24-task strict MVP average
changes from **89.208% to 89.250%**; its rounded display
remains **89% (+0 whole-number pp)**. Physical Hyprland
acceptance remains **51%** and production cutover remains
**0%**. All D-Bus and compositor tests were isolated; installed
Mako, the desktop notification bus, and Hyprland settings
were never touched.

## Source parity: Notify insertion and Reload property invalidation timing (2026-10-25)

The pinned Mako v1.11.0 code has an observable distinction between an
**inserted** notification, a **replacement**, and **configuration reload**:

1. `dbus/xdg.c:handle_notify` calls `insert_notification(state, notif)`
   only when `replaces_id != notif->id` after explicit-ID lookup and
   the stack-tag reuse check (source lines 362–383).
   `notification.c:insert_notification` always invokes
   `emit_notifications_changed(state)` (lines 487–512).
   Therefore **new** Notify inserts emit the control interface's
   `Notifications` property invalidation; existing-ID and app-scoped
   stack-tag replacements **do not**, even though their stored
   payload changes. No later unconditional property emission occurs
   in Mako's Notify handler.
2. The insertion occurs **before** applying notification criteria,
   grouping, and executing the resolved `on-notify` binding
   (`dbus/xdg.c` lines 385–430). Thus when a new notification
   immediately dismisses itself via `on-notify=dismiss`, the
   insertion invalidation precedes the resulting
   `NotificationClosed`. The close path may emit an *additional*
   property invalidation if history is enabled and `max-history > 0`.
   A `history=0` close still emits `NotificationClosed`, but
   does **not** emit this second property invalidation.
3. `dbus/mako.c:handle_reload` only calls `reload_config` and
   `reapply_config` before its method reply (lines 354–368).
   `reapply_config` destroys/rebuilds surfaces, rematches criteria
   and groups, and marks surfaces dirty (lines 302–335).
   **Neither method emits** `emit_notifications_changed`.
   A restyle is not an insertion or close event, so **successful
   Reload must not invalidate** the control `Notifications`
   property. A rejected reload remains silent as well.

Posthorn previously emitted `Notifications` invalidation **after
every Notify**, even replacements, and **after** any on-notify
binding. It also invalidated `Notifications` unconditionally
following a successful `Reload`. Both behaviors diverged from
Mako's public D-Bus signals.

**Corrections:** the notification store now reports whether
`push()` increased active cardinality while holding its mutex.
This distinguishes new insertions from either replacement form
without duplicating replacement matching logic. The freedesktop
D-Bus adapter emits an insertion invalidation **before** executing
the staged notify binding, and none for replacement. The Mako
control Reload method now reloads and reapplies normally without
emitting a spurious Notifications property change. Existing
control `SetMode`/`SetModes` behavior still emits its **Modes**
invalidation, as upstream `mode.c:set_modes` always does;
expiry and dismissal invalidations retain the separate
history-gated semantics already verified.

**Regression acceptance:** private D-Bus subscriptions test fresh
Notify, direct-ID replacement, app-scoped stack-tag replacement,
and immediately dismissed new notifications under both
`history=0` and normal history. The reload regression supplies
a temporary source config file, verifies a real rendered popup
width change under successful Reload **without a signal**,
verifies rejected Reload retains the last good config without
a signal, and confirms subsequent fresh Notify still emits
invalidation.

- [Notify implementation baa73961](https://github.com/sguzman/posthorn/commit/baa73961c657f4279dce6b8d4d213653d7c50552)
- [Insertion/replacement and on-notify regression 41075e58](https://github.com/sguzman/posthorn/commit/41075e58c9162a2dbc1390becf2850d89703d0f6)
- [Fully green CI 40216156479](https://github.com/sguzman/posthorn/actions/runs/40216156479):
  **320 library, 88 private D-Bus, all isolated headless Wayland tests**
- [Reload implementation 156f549](https://github.com/sguzman/posthorn/commit/156f54956ab16ad9293466490e78ba4fe1d6ebd6)
- [Reload private D-Bus regression c30bb50](https://github.com/sguzman/posthorn/commit/c30bb501842f8c624de49d50a31eee0d5181ff7d)
- [Fully green CI 40216477420](https://github.com/sguzman/posthorn/actions/runs/40216477420):
  **320 Rust library, 89 private D-Bus, and all isolated headless
  Wayland checks passed**

**Progress, evidence-backed:** T303 **86% → 87% (+1 pp)**
for the accepted Reload source-parity correction.
T301 and T302 remain **99%** (no extra parity credit for
additional Notify signal coverage at already near-complete tasks).
P3's exact task average is **95.000%**, previously
**94.667%** (rounded display **95% → 95%**).
The strict 24-task M0 average rises **89.250% → 89.292%**
(display remains **89%**). Production cutover remains
**0%**, physical Hyprland acceptance **51%**.
The installed Mako instance, desktop session notification name
and personal compositor configuration were never accessed.

**Remaining caveats:** these changes do not certify all asynchronous
signal interleavings or physical Hyprland behavior. Mako's
synchronous close/order semantics and the eventual opt-in
production cutover are separate acceptance gates.

## Source parity: rounded Cairo background, progress and SOURCE border (2026-10-25)

Pinned Mako v1.11.0 `render.c:render_notification` draws notification
borders in this precise Cairo order, not as two nested filled rounded
rectangles:

1. It builds a **single** rounded rectangle with
   `x = style.border_size/2`,
   `width = notification.width - style.border_size`, and the
   **original** corner radii, then fills it with the notification
   background using `cairo_fill_preserve`.
2. It copies that same path, saves the context, and clips progress
   rendering against it. Progress is a rectangle starting at
   `x = style.border_size`, with integer width
   `(notification.width - 2*style.border_size) * value / 100`
   (clamped as in source), and height
   `notification.height - style.border_size`.
   The progress-color operator may be `OVER` or `SOURCE`.
3. It restores the saved context, appends the copied original
   rounded path, sets `CAIRO_OPERATOR_SOURCE`, and strokes that
   path using the configured border color and line width.
   This is specifically required so translucent borders **replace**
   underlying colored pixels rather than alpha-mix with them.

Posthorn previously approximated that sequence by first filling
an outer border, then filling an inset radius-reduced background,
and clipping progress to the reduced rounded rectangle.
Even though opaque straight-edged borders could look correct,
the corner curvature, semi-transparent source alpha and clipped
progress edge did not necessarily match upstream Cairo.

Posthorn now paints in the pinned source order with the **original
unmodified corner radii** and shared path. The stale
`CornerRadii::inset` method was removed. Bounds checking,
layout size caps, public D-Bus behavior, and user workstation state
were not altered.

An independent raster regression constructs a reference Cairo
image following `render.c`'s background-fill / progress-clip /
SOURCE-stroke sequence for a notification with
`border-size=4`, `border-radius=17`, partially translucent
background and border colors, and `source` progress blending.
It compares **all output bytes**, and separately validates
premultiplied border alpha. The **321 library tests and 89 private
D-Bus tests** pass, and the isolated headless Wayland suite
accepts the rendered popup at all tested scales.

- [Pinned upstream `render.c:215-272`](https://github.com/emersion/mako/blob/v1.11.0/render.c#L215-L272)
- [Mako Cairo path implementation f07dc9b](https://github.com/sguzman/posthorn/commit/f07dc9b042988b49112e374b2990b3d9baea6ea3)
- [Independent full-pixel reference regression c188dcd](https://github.com/sguzman/posthorn/commit/c188dcdc499b5414409e0f4708c6e58053fb7e58)
- [Accepted green CI 40218894027](https://github.com/sguzman/posthorn/actions/runs/40218894027):
  **321 Rust library tests, 89 private D-Bus tests, all isolated
  headless Wayland compositor tests passed**
- [Remove obsolete inset-radii helper 4034087](https://github.com/sguzman/posthorn/commit/4034087abd9e58980b32b90d67b45e658e2c6708)

A proposed offset to centered Pango text was **reverted**:
`render.c:102` defines the local `border_size` used by
the centering calculation as `2 * style.border_size`. The
later `move_to` adds `style.border_size`, so those terms cancel;
Posthorn's original full-notification centering already agreed with
Mako. That invalid candidate received **zero acceptance credit**.

No machine-readable task percentage is raised solely for this
Cairo correction: P2's existing T205 measures Pango format and
action compatibility, whereas physical workstation renderer
acceptance belongs to T503 and remains unperformed. This
implementation and isolated reference regression contribute real
renderer parity evidence, but **T503 remains 51%** until actual
Hyprland work is authorized and performed. M0 stays
**89.292% raw (89% displayed)**; session ledger delta is **0 pp**.

## Source parity: unbounded Cairo glyph overhang after Pango layout (2026-10-25)

Pinned Mako v1.11.0 `render.c:318-325` calls `move_to`,
`pango_cairo_update_layout`, and `pango_cairo_show_layout`
after setting the Pango layout's width, height, wrapping,
ellipsizing and alignment. It does **not** add a separate Cairo
`rectangle(...); cairo_clip(...)` before drawing text. The
Pango layout is the formatting/ellipsizing boundary, while ink
overhangs (such as oblique or italic glyphs) can extend beyond
the layout's nominal ink rectangle and remain visible within the
underlying notification surface.

Posthorn previously created an additional Cairo rectangle clip
at `text_x, text_y, available_w, available_h` immediately
before `show_layout`. That clipped Pango glyph overhang ink
which real Mako paints. The correction removes precisely that
redundant clip without changing layout text, sizing, alignment,
notifications, other Cairo drawing or buffer safety.

A discriminating pixel regression constructs an isolated
Pango/Cairo reference for a right-aligned oblique-font
notification on a transparent surface, **without** the extra clip.
It compares all frame bytes with Posthorn's raster and renders
the old clipped variant as a negative control. The test requires
at least one glyph fixture where the old clip **actually changes
pixels**; it passed, so the correction is not merely a source
comment with an inert fixture.

This follows the earlier successful exact Mako Cairo rounded
border path and SOURCE stroke correction, but is a distinct
Pango painting behavior.

- [Pinned source: `render.c:318-325`](https://github.com/emersion/mako/blob/v1.11.0/render.c#L318-L325)
- [Renderer change e60bce0](https://github.com/sguzman/posthorn/commit/e60bce0ad3b288c613ce6df39ec3a03fb735a611)
- [Discriminating raster regression 9c52f5b](https://github.com/sguzman/posthorn/commit/9c52f5b339f932a5f5e840e8da6cacb4e77b71f2)
- [Full green CI 40220558551](https://github.com/sguzman/posthorn/actions/runs/40220558551):
  **322 Rust library tests, 89 isolated private D-Bus tests,
  and all headless Wayland compositor checks passed**.

**Evidence-backed progress:** T205 **85% → 86% (+1 pp)**,
P2's raw average **96.600% → 96.800%**
(displayed **97% → 97%**); strict 24-task M0
**89.292% → 89.333% (+0.042 pp raw)**,
displayed **89% → 89%**. This source-specific
Pango drawing acceptance does **not** close full markup,
formatting, actions, physical Hyprland, or opt-in cutover
acceptance. T503 remains **51%** and T504 **0%**.
No installed Mako, production D-Bus notification owner
or personal Hyprland configuration was accessed.

## Conditional Mako rounded-corner popup height (2026-10-25)

Pinned Mako v1.11.0 `render.c:199-213` computes the current
notification height from the clamped Pango text height, actual
rendered icon, border widths and signed vertical padding.
It then evaluates **two sequential conditionals**:

1. **Only if** the current height is **strictly less** than
   `radius_top_left + radius_bottom_left`, set it to that
   pair's radius sum **plus twice the configured border width**.
2. **Only if** the current (possibly already expanded) height is
   **strictly less** than `radius_top_right + radius_bottom_right`,
   set it to that pair's sum plus both border widths.

Posthorn previously used an unconditional
`max(measured_height, max(corner_pair_sums) + 2*border)`.
This is **not equivalent**. For `border-size=4`,
`border-radius=7`, empty text and vertical `padding=3`,
the Pango-measured height is exactly **14 px**, equal to the
corner sum. Mako leaves the result at **14 px** while the
old Posthorn logic made it **22 px**. The incorrect extra height
changes popup layout, grouping offsets, background shape and
pointer hitbox geometry. The conditionals also have meaningful
order with unequal pairs: with left radius sum 10, right sum 18,
border 4 and base height 8, Mako first expands to 18 and the
second check does nothing, rather than automatically expanding
to 26.

Posthorn now uses
`CornerRadii::adjust_height(current_height, border)` with both
source-order strict-comparison branches. The old unconditional
`required_height` calculation was removed. No safety cap,
source border-painting, notification behavior or compositor
configuration was changed.

The regression suite tests every integer boundary around the
radius sum (base heights 8, 12, 13, 14, 15, 16, 21, 22, 25),
both unequal-pair ordering cases, and end-to-end raster
measurement **and** rendering for empty notifications at
vertical padding 2, 3, 4, 6 and 7. This demonstrates the
visible geometry discrepancy, not just a helper-method
assertion.

- [Pinned Mako source: `render.c:199-213`](https://github.com/emersion/mako/blob/v1.11.0/render.c#L199-L213)
- [Conditional geometry implementation 2fe078f](https://github.com/sguzman/posthorn/commit/2fe078f603ebffe13e9d230c7c1c6d67559c59b4)
- [Asymmetric boundary and rendered height regressions cc7bc19](https://github.com/sguzman/posthorn/commit/cc7bc19b4bee224c380161dabb9fc592062acd7e)
- [Full green CI 40221519045](https://github.com/sguzman/posthorn/actions/runs/40221519045):
  **323 Rust library tests, 89 isolated private D-Bus tests,
  and the complete headless Wayland suite passed**.

**Accepted progress:** T105 **87% → 88% (+1 pp)**,
P1 raw average **91.600% → 91.800%**
(displayed **92% → 92%**), strict 24-task M0 average
**89.333% → 89.375% (+0.042 pp raw)**,
displayed **89% → 89%**. Physical Hyprland
T503 remains **51%**, and T504 production cutover
**0%**, neither credited from isolated CI.
The real Mako daemon, production notification bus and
Hyprland configuration remained untouched.


## POSIX ERE stacked repetition operators versus Rust lazy quantifiers (2026-10-25)

Mako v1.11.0 `criteria.c:apply_criteria_field` compiles
`summary~` and `body~` using glibc
`regcomp(REG_EXTENDED | REG_NOSUB)` and evaluates using
`regexec`. Native **C-locale glibc** probes exposed a
meaningful regex-language divergence from Rust `regex`:

- Glibc accepts **stacked postfix repetitions**: `a+?`
  is an optional *one-or-more* run (accepting the empty
  string), while Rust interprets `a+?` as a lazy one-or-more
  run (**not** accepting empty). Thus this discrepancy
  changes criteria matches even with `REG_NOSUB`.
- Likewise glibc accepts `a?+`, `a+*`, `a?*`,
  `a++`, and `a*+` as repeated postfix operators.
  Rust may interpret an adjacent `?` as a laziness
  modifier or reject another operator outright.
- Counted repetitions can themselves be repeated:
  `a{2}?` matches empty or two `a` bytes,
  `a{2}+` matches positive even counts,
  `a+{2}` matches at least two `a` bytes, and
  `a{2}{3}` matches exactly six. These are nested
  quantifiers in glibc, **not** malformed tokens.

Posthorn now normalizes those nested operations by
parenthesizing the **complete prior quantified atom** with
a noncapturing Rust group before applying the subsequent
quantifier. For example `a+?` becomes `(?:a+)?`,
`a{2}+` becomes `(?:a{2})+`, and `a{2}{3}`
becomes `(?:a{2}){3}`. The normalizer recognizes
literal/escaped atoms, real POSIX bracket expressions and
ordinary parenthesized groups, keeping operators inside
brackets (such as `[a+?]`) literal. It runs after the
existing POSIX bracket and GNU zero-minimum brace
normalizations, so `a{,2}+` is treated as repeated
`a{0,2}`.

The regression performs positive **and negative**
match assertions checked with native `regcomp`/`regexec`
for 15 anchored patterns: stacked optional/plus/star,
nested counted ranges, `(ab)+?`, `[ab]+?`,
multiple trailing optional operators, and zero-minimum
`{,2}` followed by `+`. It also verifies a
real `Config::parse` `summary~` selector against
empty and nonempty notification summaries and asserts
the normalized source grammar for escaped and
bracket-contained metacharacters.

- [Pinned Mako source: `criteria.c:332-349`](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Nested POSIX quantifier translator efaae9e](https://github.com/sguzman/posthorn/commit/efaae9ee4bdd5aff345082b211073e9a1825fd43)
- [Native glibc zero-minimum fixture adjustment 217ce3e](https://github.com/sguzman/posthorn/commit/217ce3e93a8e279d9b4760f18fa0deaa7b3c5cbd)
- [Character delimiter scanner type correction b9f781b](https://github.com/sguzman/posthorn/commit/b9f781b90f2b5604832f2fb5e78b27c1ca2f3a1d)
- [Full green CI 40223413158](https://github.com/sguzman/posthorn/actions/runs/40223413158):
  **324 Rust library tests, 89 private D-Bus tests, and
  complete isolated headless Wayland acceptance passed**.

The first intermediate CI test used an incorrectly classified
negative fixture for `^a{,2}+$`: native glibc does match
the empty string. The fixture was corrected before full
acceptance. Intermediate unsuccessful runs do **not**
earn parity credit.

**Accepted progress:** T103 **84% → 86% (+2 pp)**;
P1 raw average **91.8% → 92.2%**, displayed
**92% → 92%**; strict M0 average
**89.375% → 89.458% (+0.083 pp raw)**,
displayed **89% → 89%**. Physical
Hyprland acceptance remains **51%** and
opt-in cutover **0%**. This covers the tested
glibc quantifier grammar only, not full POSIX
collation, every malformed repetition edge or
unsupervised production deployment.

## glibc counted interval grammar and safe Reload rejection (2026-10-25)

Mako v1.11.0 compiles `[summary~]` and `[body~]` selectors
through `criteria.c` with glibc `REG_EXTENDED|REG_NOSUB`.
The expression's **counted repetitions** use glibc's
`RE_DUP_MAX=32767` and reject malformed syntax. Native
C-locale `regcomp` probes confirmed the following:

- `a{foo}`, `a{+2}`, `a{-2}`, `a{2,+3}`,
  `a{2,-3}`, `a{ 2 }`, `a{2.3}`,
  `a{}`, `a{2,3,4}` and decreasing
  `a{2,1}` intervals are invalid.
- An unescaped opening interval without a closing brace
  (`a{`, `a{2`) is invalid; 32768 and higher
  lower or upper bounds also fail glibc compilation.
- GNU glibc accepts omitted endpoints `a{,2}`,
  `a{,}`, `a{2,}`, leading-zero count `a{02}`,
  and valid `a{2,3}`. Escaped braces such as
  `a\\{foo\\}` and braces inside a POSIX bracket
  class remain **literal**, not interval operators.

Rust `regex` does not enforce the same glibc repetition
grammar and may interpret malformed braces as literal
characters or accept repetitions above glibc's bound.
Posthorn now validates the original POSIX interval structure
**before** calling the Rust regex compiler, including
numeric bounds, malformed content, missing closers and
descending ranges. The scanner treats escaped metacharacters
and POSIX bracket subexpressions as literals. This also avoids
building oversized Rust regex automata for source-invalid
intervals.

**Important acceptance correction:** the first integration
run failed an existing GNU word-boundary test. It revealed
that validation had initially been placed *after*
Posthorn translated glibc `\\<` and `\\>` into
Rust `\\b{start}` and `\\b{end}`.
The braces in those generated Rust assertions are **not**
glibc repetition operators. Validation was moved to
the earlier POSIX-source stage, before those translations.
The final run explicitly passes the GNU word-boundary and
interval test families together.

The new library regression tests both source-accepted and
source-rejected spellings for `summary~`, `body~`,
literal braces, bracket class braces, and valid expression
matches. It checks the numerical `32767` bound
**without attempting to compile an enormous valid
32,767-fold expression**, because the native glibc
compilation can itself be resource-intensive.

A separate private D-Bus integration test starts the
notification service from a temporary Mako configuration,
submits an active notification, and attempts four
malformed regex configuration reloads through the actual
`fr.emersion.Mako.Reload` method. Each invalid reload
must reject atomically, retain the last good global and
criteria styles, and leave the live notification
unchanged. A subsequent valid repetition rule reloads
successfully, and the next notification matches the
updated body selector.

- [Pinned upstream `criteria.c:332-349`](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [POSIX interval grammar implementation and library regression 8862918](https://github.com/sguzman/posthorn/commit/88629188369d2beed21cd41d03c2e9d2bfd448dd)
- [Private D-Bus Reload rejection/recovery test 93d9ffb](https://github.com/sguzman/posthorn/commit/93d9ffb63ab11a4b03e328214f846e6508f997cb)
- [Fix source-stage validation ordering 503bed0](https://github.com/sguzman/posthorn/commit/503bed03fdfb71fafa941e9ced7b58880b5f6eae)
- [Full green acceptance CI 40225481533](https://github.com/sguzman/posthorn/actions/runs/40225481533):
  **325 Rust library tests, 90 isolated private D-Bus
  tests, and the full headless Wayland compositor suite passed**

**Accepted progress:** T103 **86% → 87% (+1 pp)**.
P1 raw average **92.2% → 92.4% (+0.2 pp)**,
still displayed **92%**. The strict 24-task MVP
average **89.458% → 89.500% (+0.042 pp)**,
which the authoritative generated roadmap rounds
to **90%**, not 100%. This increment covers
the specified interval grammar and isolated reload
behavior only, **not complete glibc regex parity**.
Actual Hyprland acceptance T503 remains **51%**,
and opt-in reversible cutover T504 **0%**.
No installed Mako process, real notification bus
or user's Hyprland configuration was touched.

## Glibc POSIX escaped ordinary punctuation and zero in regex criteria (2026-10-25)

Pinned Mako v1.11.0 compiles `summary~` and `body~`
through glibc `regcomp(REG_EXTENDED | REG_NOSUB)` in
`criteria.c:332-349`. Native C-locale `regcomp` /
`regexec` probes establish that an escaped ordinary punctuation
character matches the character, not a Rust-specific escape
or a regex syntax error. Tested anchored examples include
`\!`, `\#`, `\@`, `\_`, `\=`, `\,`,
`\/`, `\:`, `\ `, `\.`, `\+`,
`\(`, and `\{`. GNU `\0` means a literal
ASCII **0**, not a NUL escape.

Posthorn now converts these *outside-class* escaped literals to
Rust-safe escaped/literal text using `regex::escape`. This
retains backslashes where Rust requires them (for example `\#`
and `\+`) but drops them where Rust would otherwise reject
or reinterpret the glibc expression. It deliberately leaves
bracket-class backslash membership, GNU word and buffer anchors,
and digit backreferences `\1` to `\9` on their existing
paths. Support for full POSIX backreference matching is **not**
claimed.

A new library regression covers 15 glibc-probed patterns
with positive and negative matches plus end-to-end `Config::parse`
for a `body~` rule. The initial CI attempt caught a wrong
assertion about Rust's *normalized source representation* for
`\#`; that assertion was corrected before acceptance, without
changing the semantic glibc fixtures.

- [Pinned upstream `criteria.c`](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Translator and regression ed4005c](https://github.com/sguzman/posthorn/commit/ed4005cd3cb03bccfe03f0902a2f9857737e7f50)
- [Regression assertion correction 6328444](https://github.com/sguzman/posthorn/commit/6328444d141c3a963ac5fa3daddbd9d1a7598289)
- [Full green isolated CI 40227727438](https://github.com/sguzman/posthorn/actions/runs/40227727438):
  **326 Rust library tests**, **90 private D-Bus tests**, and the
  entire headless Wayland acceptance suite passed.

**Accepted progress:** T103 **87% → 88% (+1 pp)**;
P1 raw **92.4% → 92.6% (+0.2 pp)**, displayed **92% → 93%**;
strict 24-task M0 raw **89.500% → 89.542% (+0.042 pp)**,
still **90%** displayed. Actual Hyprland T503 **51%**
and explicitly authorized reversible cutover T504 **0%**
are unchanged. No live Mako, notification bus or desktop
configuration was modified.

## Glibc unmatched closing regex delimiters are literal atoms (2026-10-25)

Pinned Mako v1.11.0 compiles `summary~` and `body~`
with glibc `regcomp(REG_EXTENDED | REG_NOSUB)`.
Native glibc C-locale checks confirm that a closing `)`, `]`,
or `}` with no corresponding opening delimiter is a **literal**
character, not a syntax error. For example, `^)$` matches `)`,
`^]$` matches `]`, `^}$` matches `}`, and `^(a))$`
matches `a)`. Those literal atoms also admit postfix repetition:
`^a)*$` accepts `a)))`, and `^a{2}}*$` accepts `aa}}`.

Posthorn now escapes only **unmatched** closing delimiters
before passing the expression to Rust regex. The source-level
normalizer tracks balanced group nesting, POSIX bracket elements,
escaped characters, and already-validated glibc repetition
intervals. It runs before GNU word-boundary rewrites add
Rust-only `\b{start}` and `\b{end}` braces, which must not
be mistaken for glibc interval closers.

Regression coverage includes 12 native-glibc-confirmed
positive/negative anchored selector fixtures, direct checks that
balanced captures and real intervals survive, and a full
`Config::parse` notification style matching case. These tests
demonstrate the specific closing-delimiter contract, not all
remaining POSIX backreference or collation semantics.

- [Pinned Mako criteria source](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Implementation and regressions fdac24e](https://github.com/sguzman/posthorn/commit/fdac24e38d4274dc3a7ab827c827964fcffd723d)
- [Fully green isolated CI 40228577605](https://github.com/sguzman/posthorn/actions/runs/40228577605):
  **327 Rust library tests, 90 isolated private D-Bus tests,
  and full headless Wayland compositor acceptance passed**.

**Accepted progress:** T103 **88% → 89% (+1 pp)**,
P1 raw **92.6% → 92.8% (+0.2 pp)** (display 93%),
strict 24-task M0 **89.542% → 89.583% (+0.042 pp)**
(display 90%). Actual physical Hyprland T503 remains 51%,
and opt-in reversible Mako cutover T504 remains 0%.
No installed daemon, production notification bus or live
Hyprland config was modified.
