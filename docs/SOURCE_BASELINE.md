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

## Glibc long zero-padded POSIX counted repetitions (2026-10-25)

Mako v1.11.0 uses glibc `regcomp(REG_EXTENDED | REG_NOSUB)`
for `summary~` and `body~` criteria in `criteria.c:332-349`.
A direct C-locale `regcomp`/`regexec` check confirmed
that the *numeric value* of a counted repetition, rather than
the length of its zero-padded spelling, is subject to
`RE_DUP_MAX=32767`. Glibc accepts a count such as
`a{0000000000000000000000000000000000002}`, plus
long-zero-padded ranged, omitted-lower, unbounded, and
stacked counts. Direct glibc probes accepted 1,025-digit
zero-padded repetitions with a small numerical value.
Values of 32768 and descending bounds remain invalid.

Posthorn's source interval validator previously rejected
more than 32 count characters. It now checks ASCII digit
syntax, removes leading zeroes before bounded numerical
conversion, and rejects actual values above 32767.
The same source-aware pass canonicalizes valid intervals
to small Rust forms, including `{,2}` → `{0,2}` and
`{0000000002}` → `{2}`. This is done outside bracket
classes and escaped brace literals, and before the
stacked-quantifier rewrite. GNU word-boundary generated
assertions remain intact.

The isolated Rust regression covers 256 zero digits in
exact counts, positive ranges, omitted and unbounded bounds,
stacked counts and their actual match behavior, as well as
malformed/too-large values, escaped class/literal braces,
and full parsed `body~` style selection. We avoid compiling
a full 32,767-fold regex; high-limit cases are validator-only.

- [Pinned Mako criteria.c](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Implementation and new regression 8d6d148](https://github.com/sguzman/posthorn/commit/8d6d148c8d2b01a5089ecdb9a8b92096cf0661b9)
- [Both CI jobs passed 40229820202](https://github.com/sguzman/posthorn/actions/runs/40229820202):
  **328 Rust library tests, 90 isolated D-Bus tests** and
  complete isolated headless Wayland acceptance.

**Accepted progress:** T103 **89% → 90% (+1 pp)**;
P1 raw **92.8% → 93.0% (+0.2 pp)**, displayed 93%;
strict 24-task M0 raw **89.583% → 89.625% (+0.042 pp)**,
displayed 90%. Physical Hyprland T503 remains **51%**,
opt-in production cutover T504 remains **0%**.
No change to live Mako, its D-Bus ownership, or desktop config.

## Glibc leading double-hyphen POSIX bracket range (2026-10-25)

Mako v1.11.0 compiles `summary~` and `body~` with glibc
`regcomp(REG_EXTENDED | REG_NOSUB)` (`criteria.c:332-349`).
Native C-locale probes confirmed that **a leading `--`**
inside a POSIX bracket expression represents a range
from literal `-` to the following endpoint: `[--a]`
matches `-`, `.`, `/`, `9`, `A`, and `a`, but not `b`.
`[--]` matches literal `-`; `[^--a]` and `[^--]`
apply corresponding negation. Rust regex instead treats
unescaped `--` as a reserved set-subtraction operator.

Posthorn's *first* bracket normalization pass now
backslash-escapes only the first hyphen of a class-starting
`--` sequence (including after leading `^`). The second
hyphen remains the source range operator; subsequent
POSIX bracket and escaped-character passes preserve the
range. Unrelated class members and real nested POSIX
elements are not transformed.

A new Rust regression exercises four native-glibc-
confirmed expressions, with positive/negative selector
matches, asserts the exact intermediate normalized syntax,
and checks a complete `Config::parse` `body~` rule.
The full existing regression and isolated compositor suites
passed. This is **not** evidence for universal hyphen-range
validation; source-invalid cases such as `[a-z--]` remain
within the unclosed overall regex-parity audit.

- [Pinned Mako `criteria.c`](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Targeted implementation and regression 379a984](https://github.com/sguzman/posthorn/commit/379a984d2c4560c7a6b6fbd271fe03b725dc3470)
- [Fully green isolated CI 40231268590](https://github.com/sguzman/posthorn/actions/runs/40231268590):
  **329 Rust library tests, 90 isolated D-Bus tests, and
  complete headless Wayland acceptance passed**.

**Accepted progress:** T103 **90% → 91% (+1 pp)**;
P1 raw **93.0% → 93.2% (+0.2 pp)**, displayed 93%;
24-task strict M0 raw **89.625% → 89.667% (+0.042 pp)**,
displayed 90%. Real Hyprland T503 remains 51%,
and explicitly authorized reversible cutover T504 remains
0%. No live daemon, notification bus or desktop config
was accessed or changed.

## Glibc all-hyphen POSIX bracket classes and following interval validation (2026-10-25)

Pinned Mako v1.11.0 compiles `summary~` and `body~`
using C-locale glibc `regcomp(REG_EXTENDED | REG_NOSUB)`.
Native probes establish that `[-]`, `[--]`, `[---]`,
and `[----]` are all legal bracket expressions matching
only literal `-`, and their leading-`^` variants match
every single-byte character **except** `-`. Five or
more hyphens alone are rejected with an invalid-range
error (`REG_ERANGE`). Rust regex reserves `--` for
class subtraction; the raw POSIX class is not equivalent.

Posthorn's dedicated source-compatible pass canonicalizes
only complete one-to-four-hyphen classes into a Rust
escaped literal hyphen, including negated classes.
It runs **after** glibc C-bracket-backslash lowering:
running it earlier would make an inserted Rust escape
look like an original source backslash and change
meaning. Real nested POSIX bracket elements and escaped
opening brackets are preserved. Classes of five or more
hyphens are rejected before attempting Rust compilation.

A related error in the existing glibc interval validator
was also closed: an escaped **first** bracket member
(such as normalized `[\-]` or `[\]]`) must consume the
first class slot. Otherwise a following closing `]`
could be mistaken for a literal leading member, hiding
an out-of-range `{32768}` repetition behind a false
in-class state. The validator now closes that class
correctly and applies `RE_DUP_MAX` to the next interval.

Direct C-locale positive/negative regression fixtures
cover one to four literal hyphens with both negations,
source-invalid five/six/ten-hyphen classes, the
post-class repetition-limit cases (including escaped
leading `]`), valid counted hyphen classes, and full
`Config::parse` `body~` matching. Existing tests remained
green. Unrelated POSIX bracket grammar cases are not
claimed complete.

- [Pinned Mako regex compilation](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Initial implementation 2ab9ca4](https://github.com/sguzman/posthorn/commit/2ab9ca46def0989455f43b4d2f1a18976134820e)
- [Pass-order correction 663524e](https://github.com/sguzman/posthorn/commit/663524e65b2eee9b36ff0537dc5c5eb53fbbf6ce)
- [Escaped-first-member validator regression 86429c5](https://github.com/sguzman/posthorn/commit/86429c5435bcb971ccca3ad6c0f01836e5174185)
- [Fully green isolated CI 40232019541](https://github.com/sguzman/posthorn/actions/runs/40232019541):
  **330 Rust library tests, 90 isolated private D-Bus tests,
  complete headless Wayland compositor suite passed**.

**Accepted progress:** T103 **91% → 92% (+1 pp)**;
P1 raw **93.2% → 93.4% (+0.2 pp)**, displayed 93%;
strict 24-task M0 raw **89.667% → 89.708% (+0.042 pp)**,
displayed 90%. Actual Hyprland T503 is still **51%**
and explicitly authorized reversible cutover T504 is
**0%**. The production daemon, notification name,
and real compositor configuration were not touched.

## Glibc invalid dash ranges and three-leading-hyphen members (2026-10-25)

Pinned Mako v1.11.0 uses `regcomp(REG_EXTENDED | REG_NOSUB)`
to compile `summary~` and `body~` in `criteria.c:332-349`.
Native **C-locale glibc** regex probes established:

- `[a--z]`, `[a-z--]`, `[-a--]`, `[----a]`,
  `[^----a]`, `[ab--c]`, `[0--9]`, and `[--a--]`
  fail to compile with an invalid range end.
- `[--a]` is a range from literal `-` to `a`: it
  matches ASCII punctuation, numbers and letters up to `a`.
- `[---a]` is **not the same range**: it matches only
  literal `-` or `a`, and rejects `.` and `0`.
  `[---a-z]` matches literal `-` or a letter from `a` to `z`.
  Their negated forms invert that membership.
- `[----]` and `[^----]` denote the singleton literal
  hyphen and its complement.

Posthorn now pre-validates only *simple* ASCII alphanumeric
and hyphen bracket classes before converting them to Rust:
the validator rejects repeated interior `--` and
four or more leading hyphens followed by an endpoint,
without rejecting valid two-/three-hyphen initial runs.
Classes with POSIX elements, escapes, or other punctuation
remain delegated to their existing source-normalization passes.

After the original C-bracket-backslash normalizer quotes
a leading double hyphen, the dash-class pass also
distinguishes exactly **three** original leading hyphens
from a two-hyphen range. It reduces `[---a]` to Rust's
`[\\-a]`, keeping any subsequent `a-z` range syntax intact.
The regression covers error and success paths through both
`summary~` and complete `Config::parse` `body~` selector
handling, with native-C positive and negative match fixtures.

The initial Rust test correctly exposed the missing literal
`-` match for `[---a]` (CI run `40233288584`, red). The
additional source normalization and expanded regression in
`38e6d19` resolved that test. Final green CI run
`40233513070` passed **331 library tests, 90 isolated
private D-Bus tests and the full headless Wayland suite**.

- [Pinned upstream compilation](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Simple source range validator 28d9ffb](https://github.com/sguzman/posthorn/commit/28d9ffbda01f44945efcaec38758fbd692d1e7ac)
- [Triple-leading-hyphen correction 38e6d19](https://github.com/sguzman/posthorn/commit/38e6d193eac743b8036c4db11a5f9e0bae5962d3)
- [Accepted both-job CI run 40233513070](https://github.com/sguzman/posthorn/actions/runs/40233513070)

**Accepted progress:** T103 **92% → 93% (+1 pp)**;
P1 raw **93.4% → 93.6% (+0.2 pp)**, displayed 94%;
strict 24-task M0 raw **89.708% → 89.750% (+0.042 pp)**,
displayed 90%. Physical Hyprland T503 still 51%,
explicitly authorized reversible cutover T504 still 0%.
The installed Mako, real notification bus and Hyprland
configuration remain untouched.

## Glibc C-locale POSIX punctuation-range parity (2026-10-25)

Mako v1.11.0 `criteria.c:332-349` compiles `summary~`
and `body~` with `regcomp(REG_EXTENDED | REG_NOSUB)`.
Native GNU glibc C-locale `regcomp`/`regexec` probes
established two distinct punctuation cases:

- An interior `--` following an ASCII class member above
  `'-'` (0x2d) is an invalid descending range end.
  Glibc rejects forms such as `[a.--z]`, `[.--z]`,
  `[a/--z]`, `[a:--z]`, `[a;--z]`, `[a<--z]`,
  `[a=--z]`, `[a>--z]`, `[a?--z]`, `[a@--z]`,
  `[a_--z]`, and `[a~--z]`.
- A double hyphen following an ordinary ASCII punctuation
  character **below** `'-'` can be a valid ascending range.
  `[!--z]` includes all bytes from `!` through `-` plus
  `z`; `[+--z]` includes `+`, `,`, `-` and `z`;
  `[a&--z]` includes `a`, the characters `&` through
  `-`, and `z`. The corresponding negated class complements
  the accepted set. Rust regex would otherwise interpret
  an unescaped `--` as class set subtraction.

The original conservative simple-class source validator
now includes the tested ASCII punctuation above `'-'`.
It does **not** reject lower-punctuation operators which
glibc accepts. A new normalization pass handles
simple classes with exactly two hyphens following
low punctuation: it preserves the first `-` as the range
operator and quotes the second for Rust regex. It runs
**after** the glibc class-backslash lowering, so its
inserted Rust escape is not reinterpreted as a literal
C-source backslash. Classes with nested POSIX elements,
source escapes, or other unsupported syntax are passed
through to the existing specialized translators.

Rust regressions include native-glibc validated positive
and negative match controls, both summary and body selectors
through `Config::parse`, and unchanged controls for the
separate leading-hyphen and POSIX collating grammars.
This does **not** claim complete glibc regex compatibility,
notably backreferences and other untested bracket grammar.

- [Pinned Mako criteria.c](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L332-L349)
- [Reject descending punctuation ranges 2b983c1](https://github.com/sguzman/posthorn/commit/2b983c16710811c03c9e8af653b7352b8798fbad)
- [Lower valid ascending punctuation ranges 3a7b28f](https://github.com/sguzman/posthorn/commit/3a7b28f2918c61c8fb97139d49b7503df56ea86f)
- [First code CI 40237083490 fully passed](https://github.com/sguzman/posthorn/actions/runs/40237083490)
- [Final code CI 40237587087 fully passed](https://github.com/sguzman/posthorn/actions/runs/40237587087):
  **332 Rust library tests, 90 isolated private D-Bus tests,
  all headless Wayland acceptance checks passed**.

**Accepted progress:** T103 **93% → 94% (+1 pp)**;
P1 raw **93.6% → 93.8% (+0.2 pp)**, displayed 94%;
strict 24-task M0 raw **89.750% → 89.792% (+0.042 pp)**,
displayed 90%. Real Hyprland T503 remains **51%**,
production cutover T504 remains **0%**.
No live Mako, real notification bus, or personal
Hyprland configuration was accessed or changed.

## Mako tilde include paths with repeated separators (2026-10-25)

Pinned Mako v1.11.0's `config.c:505-522` resolves
`include=~/...` with `mako_asprintf("%s/%s",
home, path + 2)`, where `home` is the raw `getenv("HOME")`.
This retains the HOME prefix even if the part after `~/`
begins with one or several additional slashes.

Posthorn previously used `PathBuf::from(home).join(relative)`.
For a config line `include=~//child.conf`, `relative`
begins with `/`; Rust's absolute-path join discards the
parent HOME entirely. That could read the wrong file or
reject a valid source-compatible include.

Posthorn's resolver now normalizes extra leading suffix
separators **before** joining the HOME prefix, retaining
the same Linux filesystem target as Mako. The implementation
also preserves Mako's distinction between a missing HOME
(error) and a defined but empty HOME (`/child.conf`).
Already-absolute includes continue to work without HOME.
Unrelated include precedence and recursion-safety guards
are unchanged.

Unit tests cover one-to-four tilde separators, an empty
or absent HOME, absolute and invalid relative source paths.
An end-to-end `Config::from_path` regression sets HOME
only in an isolated child test process and uses a
disposable directory with `include=~//child.conf`, verifying
both inherited child width and following root height.
No real user home or configuration files are modified.

The first implementation CI run (`40238694529`) caught
a **test-fixture encoding mistake**: the fixture had literal
backslash-`n` strings instead of line endings. The direct
resolver test had already passed. Fixture commit `a1ad782`
corrected the newlines; the fully green re-run
`40239005622` passed **334 Rust library tests, 90
isolated D-Bus tests, and complete headless Wayland QA**.

- [Pinned Mako expand_config_path](https://github.com/emersion/mako/blob/v1.11.0/config.c#L505-L522)
- [Include resolver fix de1f750](https://github.com/sguzman/posthorn/commit/de1f7502c81260c1b92706ace25492851e392032)
- [Fixture correction a1ad782](https://github.com/sguzman/posthorn/commit/a1ad7825ec06e10ed4c1ed4d706df400e1b236c8)
- [Full green acceptance CI 40239005622](https://github.com/sguzman/posthorn/actions/runs/40239005622)

**Accepted progress:** T104 **88% → 89% (+1 pp)**;
P1 raw **93.8% → 94.0% (+0.2 pp)**, displayed 94%;
strict 24-task M0 raw **89.792% → 89.833% (+0.042 pp)**,
displayed 90%. Real Hyprland T503 remains **51%**
and authorized production cutover T504 remains **0%**.
The installed Mako, real bus name, and live Hyprland
configuration remain untouched.

## Original path identity for symlinked config includes (2026-10-25)

Pinned Mako v1.11.0 `config.c:780-801` opens the
exact supplied filename with `fopen(path, "r")` and
derives its diagnostic basename from that **same path**.
A symlink alias (e.g. `alias.conf` pointing to
`actual.conf`) is therefore identified as `alias.conf`,
including when used by an `include=` from another file.

Posthorn previously canonicalized each config filename
to detect recursive includes and then also used the
canonical target for file reads, rules' `source` fields,
and error locations. This lost the caller's symlink alias
in rule origins and failed-line diagnostics.

Posthorn now retains canonical paths **only** in the
recursive-include guard. File reads, decoded line origins,
stored rule source paths, and errors use the original
supplied path. Existing recursion protection remains.
The Rust diagnostic contains the full original path
instead of only Mako's basename; the significant
source-compatible alias identity is preserved.

A new Linux/Unix regression uses a disposable symlinked
config both directly and through a parent `include=`.
It verifies effective root settings, nested criteria
source identity, invalid numeric option locations,
and non-UTF-8 active-line error locations, all attributed
to the requested alias rather than the target. No
production config or HOME files were read or changed.

- [Pinned Mako config source](https://github.com/emersion/mako/blob/v1.11.0/config.c#L780-L801)
- [Original-path fix and regression 95d29f5](https://github.com/sguzman/posthorn/commit/95d29f5ec9820544f0cd6e9a429479b55d32c43a)
- [Full green CI run 40239514396](https://github.com/sguzman/posthorn/actions/runs/40239514396):
  **335 Rust library tests, 90 isolated private D-Bus tests,
  and complete headless Wayland acceptance passed**.

**Accepted progress:** T104 **89% → 90% (+1 pp)**;
P1 raw **94.0% → 94.2% (+0.2 pp)**, displayed 94%;
strict 24-task M0 raw **89.833% → 89.875% (+0.042 pp)**,
displayed 90%. Physical Hyprland T503 remains **51%**
and opt-in production cutover T504 remains **0%**.
The real notification name, installed Mako and personal
Hyprland config remain untouched.

## NUL-first physical Mako config lines (2026-10-25)

In pinned Mako v1.11.0, `config.c:801-817` reads
physical config bytes using `getline()`, strips only
leading ASCII spaces/tabs, and then skips a line if
its first visible byte is NUL (`'\0'`), a newline,
or `#`. A NUL-first line is not an active option;
any undecodable bytes after that leading NUL are
neither parsed nor rejected by Mako.

Posthorn's file decoder already ignored comments
with invalid UTF-8, but incorrectly attempted to
decode a NUL-first line with the same bytes. It
would reject a valid configuration that Mako ignores.
Posthorn now recognizes both ASCII-indented and
unindented NUL-first physical lines before UTF-8
decoding. The inline `Config::parse` path likewise
skips first-visible-character NUL lines, while
**non-UTF-8 in an active setting remains a hard error**
rather than being silently replaced.

An isolated regression creates a disposable child
config containing two ignored NUL-first binary
lines, followed by valid root and criteria options.
It verifies those settings, the correct physical
line-number attribution for the criteria section,
and nested `include=` parsing. It also checks
inline NUL-first lines and rejection of malformed
UTF-8 in a genuinely active option.

This is a narrow source-compatible case; it does
**not** establish universal compatibility for
embedded NUL bytes inside nonempty option values.

- [Pinned Mako config.c skip logic](https://github.com/emersion/mako/blob/v1.11.0/config.c#L801-L817)
- [NUL-first config compatibility efec1cf](https://github.com/sguzman/posthorn/commit/efec1cfc93aaa9eaccbcef74124674965347ce0a)
- [Fully green isolated CI 40240199124](https://github.com/sguzman/posthorn/actions/runs/40240199124):
  **336 Rust library tests, 90 private D-Bus tests,
  complete headless Wayland QA passed**.

**Accepted progress:** T104 **90% → 91% (+1 pp)**;
P1 raw **94.2% → 94.4% (+0.2 pp)**, displayed 94%;
24-task strict M0 raw **89.875% → 89.917% (+0.042 pp)**,
displayed 90%. T503 physical Hyprland remains 51%
and T504 opt-in production cutover remains 0%. The
real notification daemon, bus name and user's
Hyprland configuration were not touched.

## Mako option values with embedded C NUL terminators (2026-10-25)

Pinned Mako v1.11.0 `config.c:801-877` uses
`getline()` to obtain a physical configuration line
but then calls `strchr` and the ordinary C option
parsers on the resulting **NUL-terminated string**.
For ordinary `key=value` lines, bytes following
the first embedded NUL are not parsed. This also
means undecodable bytes after the NUL must not
invalidate an otherwise valid option.

Posthorn previously decoded **every byte** of each
active physical line as UTF-8 and used the entire
Rust string for value validation. A valid glibc
`width=472\0[non-UTF-8 bytes]` line incorrectly failed.

The byte decoder now preserves each ordinary
option's original prefix **through its first NUL**
and ignores later hidden bytes. The line parser
still strips ASCII outer whitespace **before**
truncating at that NUL. This ordering is important:
`width=472 \0ignored` is invalid in upstream Mako
because the space before NUL remains part of the
value and `strtol` does not consume trailing space.
An active invalid byte before the C terminator
remains an explicit error. Initial `[` section
lines remain untouched; their closing-bracket
recognition uses the physical line length and
requires separate source semantics.

An isolated binary-file regression covers nested
`include=`, an ordinary option whose trailing
bytes are undecodable, subsequent options, direct
`Config::parse` input, the pre-NUL whitespace
negative case, and invalid UTF-8 before the NUL.

- [Pinned Mako config.c getline and strchr](https://github.com/emersion/mako/blob/v1.11.0/config.c#L801-L877)
- [Embedded NUL option fix a93cee8](https://github.com/sguzman/posthorn/commit/a93cee82e180985fa3a6380ed5e5cefe472efb8b)
- [Fully green isolated CI 40240716085](https://github.com/sguzman/posthorn/actions/runs/40240716085):
  **337 Rust library tests, 90 private D-Bus tests,
  and complete headless Wayland QA passed**.

**Accepted progress:** T104 **91% → 92% (+1 pp)**;
P1 raw **94.4% → 94.6% (+0.2 pp)**, displayed 95%;
strict 24-task M0 raw **89.917% → 89.958% (+0.042 pp)**,
displayed 90%. T503 physical Hyprland stays 51%,
T504 opt-in production cutover stays 0%. Nothing
changed the user's installed Mako, production D-Bus
name, or live Hyprland configuration.

## Mako physical section closer with NUL-terminated selector (2026-10-25)

In Mako v1.11.0 `config.c:801-848`, physical config
lines are read with `getline()` and stripped of ASCII
indentation/trailing spaces and tabs. The section
branch checks the **physical** first `[` and final
`]` characters. It then passes the bytes between
them into `strndup()`. Because `strndup` stops at the
first NUL, the criteria parser cannot see any bytes
following an embedded NUL even when the physical
closing bracket occurs later in the same line.

Posthorn previously treated the entire region
between the physical brackets as Rust selector text,
rejecting lines such as
`[category=success\0<non-UTF-8 garbage>]`.
It now preserves the physical closing-bracket check
while lowering only a genuinely complete section
header to its C-visible selector prefix. The byte
decoder uses this rule before UTF-8 validation, so
invalid bytes after the NUL in a syntactically
complete section do not cause a false decoding error.
Likewise, `Config::parse` now truncates a complete
section's selector at the C terminator. An opening
bracket **without** a physical closing bracket remains
invalid even if the prefix before NUL looks complete.

The regression exercises a disposable nested file
containing valid root options, a NUL-suffixed section
with invalid UTF-8 hidden after its terminator,
a subsequent criteria style, and physical source
line-number preservation. It additionally tests
equivalent inline parsing and rejection of missing
physical closing brackets.

This is a targeted C-string translation. Unrelated
invalid embedded-NUL line and regex syntax remain
subject to the established source acceptance gates.

- [Pinned Mako `config.c` section parsing](https://github.com/emersion/mako/blob/v1.11.0/config.c#L801-L848)
- [Implementation 970301d](https://github.com/sguzman/posthorn/commit/970301d4a5594430b898dce332396f99171dce68)
- [Fully green isolated CI 40242070137](https://github.com/sguzman/posthorn/actions/runs/40242070137):
  **338 Rust library tests, 90 private D-Bus tests,
  complete headless Wayland acceptance passed**.

**Accepted progress:** T104 **92% → 93% (+1 pp)**;
P1 raw **94.6% → 94.8% (+0.2 pp)**, displayed 95%;
strict 24-task M0 raw **89.958% → 90.000% (+0.042 pp)**,
displayed 90%. Real Hyprland T503 remains **51%**;
explicitly authorized reversible cutover T504 remains
**0%**. No live Mako, production bus, or personal
Hyprland configuration was accessed or changed.

## Empty HOME and XDG_CONFIG_HOME default-config path semantics (2026-10-25)

Pinned Mako v1.11.0 `config.c:743-760` builds its
implicit config candidates by string concatenation:
`"%s/.mako/config"` with `getenv("HOME")`, then
`"%s/mako/config"` with `XDG_CONFIG_HOME`, defaulting
to `"%s/.config"` with HOME when XDG is unset
**or an empty string**.

Mako distinguishes absent HOME (no default file)
from a defined but empty HOME. With `HOME=""`,
its implicit legacy and fallback paths are
`/.mako/config` and `/.config/mako/config`.
Posthorn formerly used an empty Rust `Path` as the
join base and incorrectly searched relative files
`.mako/config` and `.config/mako/config` in the
current working directory.

Posthorn now constructs candidates using `/` as
the *path base* only when HOME is defined but empty.
An explicitly empty `XDG_CONFIG_HOME` also falls
back to HOME's `.config`, matching Mako's
`config_home[0] == '\0'` check. Nonempty XDG paths
remain literal, including supported relative paths.
The existing legacy-first lookup order and
`XDG_CONFIG_DIRS` omission are unchanged.

Deterministic tests verify both empty-HOME candidates,
nonempty HOME paths, an explicit nonempty XDG path,
an explicitly empty XDG value, and a relative XDG
path. They inspect only the constructed candidate
names, without reading or writing config files at
the host filesystem root. The pre-existing temporary
directory precedence test remains green.

- [Pinned Mako default path discovery](https://github.com/emersion/mako/blob/v1.11.0/config.c#L743-L777)
- [Empty HOME candidate correction eb45d27](https://github.com/sguzman/posthorn/commit/eb45d27b60930c84e0e960ca9e661ace04b5d64e)
- [Empty XDG_CONFIG_HOME fallback correction 8b3822f](https://github.com/sguzman/posthorn/commit/8b3822f1c8201efb0435201bd8329121472b9d02)
- [Full green CI 40243488785](https://github.com/sguzman/posthorn/actions/runs/40243488785):
  **339 Rust library tests, 90 isolated private D-Bus
  tests, and complete headless Wayland acceptance passed**.

**Accepted progress:** T104 **93% → 94% (+1 pp)**;
P1 raw **94.8% → 95.0% (+0.2 pp)**, displayed 95%;
strict 24-task M0 raw **90.000% → 90.042% (+0.042 pp)**,
displayed 90%. Physical Hyprland T503 remains
51% and the expressly authorized cutover T504
remains 0%. No live Mako, session bus, or user's
Hyprland configuration was touched.

## Makoctl menu C-string selected-title boundary (2026-10-25)

Pinned Mako v1.11.0 `makoctl.c:674-783` launches the
configured menu process, prints notification action
titles to its stdin, reads exactly one selection with
POSIX `getline()`, removes a terminal LF (but not a
preceding CR), and compares that selection against
titles using C `strcmp(title, selected_title)`.

`getline()` may return bytes containing an embedded NUL.
The following `strcmp()` sees **only the bytes before
the first NUL**, regardless of any later bytes, including
invalid UTF-8. Posthorn's `choose_from_menu()` already
reads only the first line and preserves CR, but its
Rust `str::from_utf8` incorrectly decoded the invisible
suffix. Thus an otherwise valid `Open\0<invalid UTF-8>`
selection would be rejected, unlike the pinned Mako.

`mako_menu_selected_line()` now extracts the first
physical line and then its C-visible prefix before NUL.
The existing subprocess behavior and strict label-to-key
comparison are unchanged. Regressions exercise a real
shell menu command producing `Open`, NUL, invalid
UTF-8, and LF; a NUL at the very start (which must
not match a nonempty action title); direct byte controls
for CR, LF, and trailing output; and invalid UTF-8
**before** NUL (which remains an error).

This is specifically `makoctl menu` selection parsing,
not a claim of complete command-line parity or
workstation action activation acceptance.

- [Pinned Makoctl menu comparison](https://github.com/emersion/mako/blob/v1.11.0/makoctl.c#L674-L783)
- [Source-parity fix 9e09001](https://github.com/sguzman/posthorn/commit/9e0900191fda9e0d5cdb3f1b8e7d1de83283e809)
- [Fully green CI 40244170082](https://github.com/sguzman/posthorn/actions/runs/40244170082):
  **339 Rust library tests, 90 private D-Bus tests,
  complete isolated headless Wayland acceptance passed**.

**Accepted progress:** T501 **95% → 96% (+1 pp)**;
P5 raw **55.75% → 56.00% (+0.25 pp)**, displayed 56%;
strict 24-task M0 raw **90.042% → 90.083% (+0.042 pp)**,
displayed 90%. Physical Hyprland T503 remains
**51%** and explicitly authorized reversible
production cutover T504 remains **0%**.
No production Mako service, real D-Bus name, or
personal Hyprland configuration was touched.

## GNU getopt POSIXLY_CORRECT option-order parity (2026-10-25)

Pinned Mako v1.11.0 `makoctl.c:83-202,468-492,674-694,810-882`
uses glibc `getopt()` / `getopt_long()` for `dismiss`,
`invoke`, `menu`, `list`/`history`, and `mode`.
When `POSIXLY_CORRECT` is absent, GNU option permutation can parse
a recognized option after a positional argument. When the variable
is defined (even as an empty string), glibc instead stops processing
options at the first positional argument and leaves all subsequent
arguments untouched.

Posthorn previously always permuted. For example, with
`POSIXLY_CORRECT=1`, Mako `mode -s quiet -a urgent` treats
`-a` and `urgent` as literal replacement mode names rather than
rejecting a mixed `-s`/`-a` invocation. Similarly,
`menu picker -n7` passes `-n7` to the menu helper rather than
using notification ID 7. The equivalent GNU-permuting forms retain
their previous behavior.

`parse_command()` now selects the ordering policy from the variable
and delegates to a deterministic parser helper, so regressions can
exercise both policies without mutating process-wide environment
variables. Direct C/glibc `getopt` probes confirm the command
interpretations for `mode`, `menu`, `invoke`, `dismiss`,
and `list`. The regression also preserves explicit `--` handling.
Unrelated command implementation and the private test bus are unchanged.

- [Pinned Mako getopt uses](https://github.com/emersion/mako/blob/v1.11.0/makoctl.c)
- [Implementation de59166](https://github.com/sguzman/posthorn/commit/de59166afb4fae5d2867c1930f6df217fdd5c5f8)
- [Fully green isolated CI 40245482127](https://github.com/sguzman/posthorn/actions/runs/40245482127):
  **340 Rust library tests, 90 private D-Bus tests, complete
  headless Wayland acceptance passed**.

**Accepted progress:** T501 **96% → 97% (+1 pp)**;
P5 raw **56.00% → 56.25% (+0.25 pp)**, displayed 56%;
strict 24-task M0 raw **90.083% → 90.125% (+0.042 pp)**,
displayed 90%. Physical Hyprland T503 remains **51%**
and explicitly authorized production cutover T504 remains **0%**.
The installed Mako daemon, real D-Bus name, and personal
Hyprland configuration were not accessed or changed.

## Makoctl CLI diagnostic text and usage newline parity (2026-10-25)

Pinned Mako v1.11.0 `makoctl.c:122-128, 695-697, 865-881,
942-954, 980-982` emits specific diagnostic strings on argument
conflicts and unknown commands. The previous Posthorn parser emitted
generic descriptions, changing user-visible stderr used by shell
scripts and interactive troubleshooting.

Posthorn now distinguishes `-a` with `-g` from a nonzero `-n`
combined with either flag, and mirrors the upstream messages
`-a and -g cannot be used together` and
`-n cannot be used with -a or -g`. The chosen argument ID still
determines the conflict, so an explicit `-n0` remains compatible.
The `mode` conflicts and invalid positional modes now emit Mako's
exact wording; a missing `menu` command and unknown commands do too.

Mako's no-argument usage path calls `fprintf(stderr, "%s", usage)`,
not an additional line-printing operation. Posthorn's bare-CLI
failure now prints its already-terminated usage exactly once,
without a second trailing newline. The explicit `help` path
continues to print usage on stderr with a successful exit. The
development-only branding and private bus remain intentionally
unchanged; this fix does **not** authorize production D-Bus ownership.

A direct parser regression asserts nine diagnostic cases plus
explicit `-n0` acceptance. An actual `posthornctl` subprocess
regression exercises six failing invocations and exact stderr
line endings with `DBUS_SESSION_BUS_ADDRESS` pointed at a
nonexistent socket. The synthetic tests do not access the user's
installed notification service. Both CI jobs passed:

- [Pinned upstream Makoctl implementation](https://github.com/emersion/mako/blob/v1.11.0/makoctl.c)
- [Diagnostic implementation 9cd4ce6](https://github.com/sguzman/posthorn/commit/9cd4ce6bbed647291bdae94f285dd3811184546b)
- [Usage newline implementation 8d974df](https://github.com/sguzman/posthorn/commit/8d974df90ee9f610512984931d6f1c9b811c2eb7)
- [Executable regression and final assertion fix 76c8579](https://github.com/sguzman/posthorn/commit/76c8579ad28677df98db7417e307dd6cc3bfab16)
- [Fully successful Rust + Wayland CI 40246620433](https://github.com/sguzman/posthorn/actions/runs/40246620433):
  **341 Rust library tests, 90 private D-Bus tests,
  and the complete isolated headless Wayland acceptance suite**.

**Accepted progress:** T501 **97% → 98% (+1 pp)**;
P5 raw **56.25% → 56.50% (+0.25 pp)** (Python-generated
display stays 56% at the tie); M0 strict 24-task raw
**90.125% → 90.167% (+0.042 pp)** (display stays 90%).
Actual Hyprland T503 remains **51%**, and authorized
reversible production cutover T504 remains **0%**.
No installed Mako, live bus, or personal Hyprland setup
was accessed or modified.

## Makoctl menu diagnostic precedence and C-visible failure text (2026-10-25)

Pinned Mako v1.11.0 `makoctl.c:590-783` emits its own
capitalized error strings and checks for failed `getline()` before
the following `waitpid()`. An empty helper stdout therefore reports
`No action selected` even if that menu helper also exited nonzero.
When the helper returned a selected line, a nonzero wait status instead
reports `Menu failed with exit code N` or
`Menu failed with signal N` before comparing the label against
notification actions. An unmatched title reports `Action not found`.
The earlier Posthorn implementation waited first, returned a generic
exit-status string, and added a `posthornctl:` prefix to all errors.

Posthorn now records those source-level distinctions with a typed
menu diagnostic that bypasses the generic development prefix.
`ListNotifications` search reports `Notification not found`;
an actionless notification reports `Notification has no actions`.
The helper's first physical line and C NUL comparison remain as
previously accepted. An invalid UTF-8 prefix before NUL cannot equal
a valid D-Bus action label, so it now reports `Action not found`
instead of a Rust UTF-8 decoding diagnostic. The child is still
reaped on empty output, avoiding a zombie while preserving the same
observable error precedence. No production bus name or daemon
configuration is changed.

A synthetic shell-based regression covers empty helper output with
exit code 7, a valid selected title followed by exit code 7, a
nonmatching selected title, invalid selection bytes, and selection
followed by process termination with SIGTERM. It also checks the
source diagnostic strings. The final synthetic PID fixture was
corrected after an intermediate failed CI run caused by an invalid
one-dollar shell signal target; that intermediate failure was not
counted as acceptance.

- [Pinned Makoctl menu implementation](https://github.com/emersion/mako/blob/v1.11.0/makoctl.c#L590-L783)
- [Menu selection correction 5474576](https://github.com/sguzman/posthorn/commit/54745760a5ab8656a4424f05e78cb87f33d58457)
- [Exact CLI menu error rendering fcd55d9](https://github.com/sguzman/posthorn/commit/fcd55d9357f6e7b11e941f40006c5277b585db49)
- [Final passing synthetic regression 207cbbc](https://github.com/sguzman/posthorn/commit/207cbbc2cf991345a7d4695a57570b54aab7ec78)
- [Full successful implementation CI 40248400777](https://github.com/sguzman/posthorn/actions/runs/40248400777):
  **342 Rust library tests, 90 isolated private D-Bus tests,
  complete headless Wayland acceptance**.

**Accepted progress:** T501 **98% → 99% (+1 pp)**;
P5 raw **56.50% → 56.75% (+0.25 pp)** (display **57%**);
strict 24-task M0 **90.167% → 90.208% (+0.042 pp)** (display
**90%**). The history/control feature inventory remains at
**96%**. Private CI confirms the synthetic helper error
behavior and existing D-Bus/Wayland regressions; it is **not**
complete physical Hyprland or actual application menu activation
parity. T503 remains **51%**, T504 remains **0%**. The installed
Mako daemon, workstation bus, and personal Hyprland config remain
untouched.

## Pinned Mako post-multiply progress-width clamp (2026-10-25)

Pinned Mako v1.11.0 `render.c:215,239-245` computes the
progress rectangle width as
`(notification_width - 2 * border_size) * progress / 100`,
then checks the result against the larger
`notification_width - border_size` background width.
If the provisional result exceeds that background width, the
upstream renderer resets the width to the original interior width.
It does **not** clamp `progress` to 100 before multiplication.

Posthorn formerly preclamped sender progress percentages, so
the Cairo path geometry for values slightly above 100 diverged.
For width 120, border 4, upstream widths for percentages
100, 101, 102, 103, 104, 105 are respectively
112, 113, 114, 115, 116, 112 logical pixels. The last
border stroke may mask some of this extra width; the correction
is an exact-path geometry fix and is **not** counted as additional
visual or physical compositor acceptance.

A source-derived regression validates percentages -20, -1, 0, 1,
50, 99, 100, 101 through 105 and 170 at a nonzero border,
plus the borderless 100/101 boundary. Existing pixel and
headless compositor tests remain intact.

- [Pinned Mako render.c](https://github.com/emersion/mako/blob/v1.11.0/render.c#L215-L245)
- [Source-compatible implementation 17269ad](https://github.com/sguzman/posthorn/commit/17269ad4ecfdfb2ca6a522880eb68de74bf661ec)
- [Accepted full Rust, private D-Bus and headless Wayland CI 40250367748](https://github.com/sguzman/posthorn/actions/runs/40250367748):
  **343 Rust library tests and 90 private D-Bus tests passed**.

An earlier attempted close/expiration-grouping change (commits
`e57119f`, `bd6ef88`, `d3b1c7c`) relied on
`group-by` inside `[group-index]`/`[anchor]` sections.
Mako's actual `validate_criteria` forbids that configuration,
so the attempted change was reverted at
[`0bf47aa`](https://github.com/sguzman/posthorn/commit/0bf47aafb6d464dc2034573dab36d4bba5a88677)
and the full revert suite passed
[CI 40249938901](https://github.com/sguzman/posthorn/actions/runs/40249938901).
The reverted experiment earns **zero** progress credit.

**Accepted progress:** T105 **88% → 89% (+1 pp)**,
P1 raw **95.00% → 95.20% (+0.20 pp)**,
strict M0 across 24 tasks **90.208% → 90.250%
(+0.042 pp)**. Displayed M0 stays 90%, P1 stays 95%.
This does not complete T402 or establish physical Hyprland
acceptance; installed Mako and the real notification bus are untouched.

## Oversized Mako border-size preserves source rectangle geometry (2026-10-25)

Pinned Mako v1.11.0 `config.c:584-585` permits any
nonnegative signed `border-size` independently of the configured
notification `width` or `height`. Its
`render.c:102,139-145,199,215-225,260-269` uses that original
configured value directly for text bounds, notification height,
the background centerline path and the final Cairo border stroke.
It never clamps the border to half the notification width or height.

Posthorn previously truncated border size independently in two
places: text/height measurement used `min(border,width/2)`, and
painting used `min(border,width/2,height/2)`. This changed the
height and visible outline of a valid thick-border notification.
Both limits have been removed. The existing maximum physical SHM
allocation size remains enforced; this is an exact style/layout
correction, not an unbounded memory allocation allowance.

A source-derived fixture uses `width=200`, `height=90`,
`padding=0`, `border-size=110`, `format=%s`, no icon,
and empty summary/body. The source's **signed** Pango text-height
limit is `90 - 2*110 = -130`, so Mako clamps the empty text
height to `-130` and calculates final popup height as
`-130 + 2*110 = 90`. The resulting popup is therefore
**200×90 logical pixels**, with the full 110px border still passed
to Cairo for the path and stroke. The earlier 200×220 assertion
was incorrect: it arose from Posthorn clamping the signed text
height prematurely and was accepted by CI before a direct source
comparison exposed the discrepancy. The corrected regression
asserts dimensions at both 1× and 2× scales.

- [Pinned Mako config.c and render.c](https://github.com/emersion/mako/blob/v1.11.0/render.c#L99-L225)
- [Accepted implementation and final test 5bafb1d](https://github.com/sguzman/posthorn/commit/5bafb1d4ebcd83cb550df851eda9729572b0b375)
- [Source-corrected signed-height regression 0133812](https://github.com/sguzman/posthorn/commit/01338124d864c44918ca3a79c0c42b51102f17e7)
- [Fully green Rust / D-Bus / isolated Wayland CI 40250768590](https://github.com/sguzman/posthorn/actions/runs/40250768590):
  **344 Rust library tests, 90 private D-Bus tests,
  complete headless compositor suite**.

**Accepted progress:** T105 **89% → 90% (+1 pp)**,
P1 raw **95.20% → 95.40% (+0.20 pp)**,
strict 24-task M0 **90.250% → 90.292% (+0.042 pp)**.
No new acceptance is claimed for T402 at 99%, real Hyprland
at T503 51%, or opt-in production cutover T504 0%.
Installed Mako and personal Hyprland configuration remain untouched.

## Output-scoped styling of hidden placeholders on mapped single surfaces (2026-10-25)

Mako v1.11.0 `render.c:443-446` creates a synthetic
hidden-notification object for an overflowing surface and explicitly
sets `hidden_notif->surface = surface` **before** calling
`apply_each_criteria`. Its output selector
(`criteria.c:149-152`) then reads the associated surface's
actual `surface_output->name` when `wl_surface.enter` has
occurred. A valid `[hidden output=DP-1]` rule may therefore
change the synthetic overflow indicator only on DP-1.
When the surface output is unknown, that same rule must not match.
Mako allows `hidden` combined with `output`, unlike non-surface
selectors such as urgency or expiring.

Posthorn's multi-surface snapshot already forwarded the per-surface
output. The earlier single-surface `snapshot_frame` path, however,
unconditionally passed `None` into the synthetic hidden style
projection, discarding the already-known output from
`SharedRuntime::surface_output`. This incorrectly suppressed
`[hidden output=DP-1]` even though normal notifications had
received their compositor observation. The single-surface path now
passes `Some(Some(observed_name))` only when an output has actually
been observed; it retains unknown-output semantics otherwise.
No production compositor, config or notification bus was touched.

A regression exercises the real Cairo-rendered placeholder with a
valid `[hidden]` fallback background and later
`[hidden output=DP-1]` override, confirms the override is painted
after a synthetic `set_surface_output(Some("DP-1"))`, and confirms
the fallback returns after the output is explicitly unknown.
Ordinary user notifications remain separate from the synthetic
overflow object, and the screen-independent test does not claim
real Hyprland or output-hotplug acceptance.

- [Pinned Mako render.c](https://github.com/emersion/mako/blob/v1.11.0/render.c#L440-L449)
- [Pinned Mako criteria.c](https://github.com/emersion/mako/blob/v1.11.0/criteria.c#L144-L153)
- [Accepted source-parity fix 29b7551](https://github.com/sguzman/posthorn/commit/29b75512794ed16bcb4d75346743cb882d911ef8)
- [Full successful Rust / private D-Bus / headless Wayland CI 40253108514](https://github.com/sguzman/posthorn/actions/runs/40253108514):
  **345 Rust library tests, 90 private D-Bus tests,
  complete isolated compositor acceptance**.

An earlier trial changed internal hidden-notification urgency and
requested-timeout defaults. Although these differ from Mako's
`calloc`-initialized synthetic object, the trial incorrectly assumed
that non-surface `[urgency]` or `[expiring]` user criteria could
reach `hidden=true`. Upstream `parse_criteria` implicitly sets
`hidden=false` unless explicitly selected, and
`validate_criteria` forbids mixing `hidden=true` with
non-surface selectors. The unobservable trial was reverted at
[`90edade`](https://github.com/sguzman/posthorn/commit/90edade816b9db4d96e3301de996ccc824459119).
It earns **no** progress credit.

**Accepted progress:** T105 **90% → 91% (+1 pp)**;
P1 raw **95.40% → 95.60% (+0.20 pp)**;
strict 24-task M0 raw **90.292% → 90.333%
(+0.042 pp)**. T401/T402 remain at 99%,
T503 physical Hyprland QA remains 51% and
T504 opt-in production cutover remains 0%.

## Pinned Mako zero-sized rounded path suppression (2026-10-25)

Pinned Mako v1.11.0 `render.c:38-41` begins
`set_rounded_rectangle` with a guard: if its width or height is
exactly zero, it immediately returns **without modifying the
current Cairo path**. This affects the background, progress clip,
and final SOURCE-mode border stroke, all of which use a copy of
the same path. It is observable for a valid `width=48`,
`border-size=48` config, where
`notif_background_width = width - border-size = 0`.
The remaining configured height, radius and border color do not
authorize painting a rounded path with a zero width.

Posthorn previously unconditionally emitted its four Cairo arcs,
even if `w == 0` or `h == 0`. It now performs the same exact
early return. Its normal nondegenerate rectangle path is unchanged.
The regression renders an empty notification with width 48,
height 96, padding 0, border-size 48 and border-radius 9. It
asserts a 48×96 framebuffer with **every alpha byte zero**.
It then reduces border-size to 2 and asserts a visible opaque
border, ensuring the guard only affects the zero-dimension case.

- [Pinned Mako render.c:38-41](https://github.com/emersion/mako/blob/v1.11.0/render.c#L38-L41)
- [Accepted correction and regression 961b9f2](https://github.com/sguzman/posthorn/commit/961b9f224915efecb78ee1b7409c51da8379a292)
- [Full successful Rust / private D-Bus / isolated Wayland CI 40255300005](https://github.com/sguzman/posthorn/actions/runs/40255300005): **346 Rust library tests, 90 private D-Bus tests, and complete headless compositor acceptance**.

**Accepted progress:** T105 **91% → 92% (+1 pp)**,
P1 raw **95.60% → 95.80% (+0.20 pp)**,
strict 24-task M0 **90.333% → 90.375%
(+0.042 pp)**. T402 remains 99%, T503
physical Hyprland QA remains 51%, and T504
production cutover remains 0%. No real notification
service, user configuration, audio routing or live bus was changed.

## Signed Pango height limits and oversized Mako borders (2026-10-25)

Mako v1.11.0 `render.c:139-155` constructs signed Pango text
layout dimensions by subtracting twice the configured border size,
padding, and any icon offsets from the configured width/height.
It passes those signed values **without clamping to a 1px minimum**.
After querying the Pango layout, `render.c:192-199` computes
`text_height = min(buffer_text_height / scale, text_layout_height)`,
even if that cap is **negative**, and adds both borders and
padding to obtain the popup's logical height.

Posthorn previously clamped both available Pango dimensions to
`1..4096` while measuring, and applied another 1px minimum
during painting. This changed valid notifications whose configured
height or width was smaller than their surrounding border/padding.
The new implementation preserves signed Pango dimensions, with
arithmetic saturation restricted to the safe Pango i32 range.
An empty-text `height=40, border-size=30, padding=0`
notification has `text_layout_height = -20`, yielding
`-20 + 60 = 40` logical pixels. A companion source
fixture corrects the earlier wrongly accepted
`height=90, border-size=110` assertion from 220 to **90**.
The tests also exercise 1x and 2x physical rasters.

- [Pinned Mako render.c:139-155,192-199](https://github.com/emersion/mako/blob/v1.11.0/render.c#L139-L199)
- [Signed text layout correction ce4e61b](https://github.com/sguzman/posthorn/commit/ce4e61b95ade36186b3a60c46dc0f7b88da6bb33)
- [Corrected older border regression 0133812](https://github.com/sguzman/posthorn/commit/01338124d864c44918ca3a79c0c42b51102f17e7)
- [Accepted full Rust / private D-Bus / isolated Wayland CI 40256739010](https://github.com/sguzman/posthorn/actions/runs/40256739010):
  **347 Rust library tests, 90 private D-Bus tests, and the complete
  headless compositor acceptance suite passed**.

The prior 200×220 source assertion was **incorrect** even though
the old CI was green. It must not be cited as Mako parity evidence.
The thick-border no-half-width-clamp change remains relevant;
the now accepted signed text-limit behavior restores the correct
joint measurement and rendering semantics.

**Accepted progress:** T105 **92% → 93% (+1 pp)**;
P1 raw **95.80% → 96.00% (+0.20 pp)**;
strict M0 (24 tasks) raw **90.375% → 90.417%
(+0.042 pp)**. T503 physical Hyprland acceptance
remains 51% and T504 opt-in production cutover remains 0%.
No production notification bus or personal desktop was modified.

## Category sound-hook direct replacement across quiet mode (2026-10-25)

Mako v1.11.0 `dbus/xdg.c:106-113` resets an existing notification
object when a `Notify` call supplies its active `replaces_id`.
It then rematches category and active modes, regroups and runs
`notification_execute_binding` for **each new Notify call**
(`dbus/xdg.c:385-430`). A Mako sound integration is a configured
`on-notify=exec` shell command, not a new built-in audio subsystem
(`notification.c:381-435`). On the other hand, `SetMode`
(`dbus/mako.c:337-350`) and `SetModes` reapply criteria to
existing notifications **without executing on-notify**.
Valid `[mode=quiet]\non-notify=none` must suppress category
commands without discarding the notification.

A new isolated private D-Bus regression exercises an active
notification's ID across these stages, each via an actual
`org.freedesktop.Notifications.Notify` call:
(1) initial `category=success` writes `success:ID` to a
temporary command-output file; (2) explicit replacement with
`category=warning` writes `warning:ID`, retaining the ID;
(3) quiet mode via private `SetMode` suppresses a new replacement's
sound; (4) leaving quiet mode via `SetMode` cannot replay that
suppressed sound; (5) a subsequent warning replacement resumes the
category sound exactly once, still with the original ID.
The active store remains at one notification across replacements.
The test uses a local file-backed command, not a paid audio service,
installed Mako, speakers, or the production bus.

- [Pinned Mako notification replacement and hook ordering](https://github.com/emersion/mako/blob/v1.11.0/dbus/xdg.c#L106-L113)
- [Pinned Mako notify binding execution](https://github.com/emersion/mako/blob/v1.11.0/dbus/xdg.c#L385-L430)
- [Pinned Mako mode reapply without hooks](https://github.com/emersion/mako/blob/v1.11.0/dbus/mako.c#L302-L350)
- [Accepted private D-Bus sound/quiet regression 885671a](https://github.com/sguzman/posthorn/commit/885671a7c457710546a0235a084042fe259fc31d)
- [Accepted complete CI 40258277057](https://github.com/sguzman/posthorn/actions/runs/40258277057):
  **347 Rust library tests, 91 private D-Bus tests and the
  complete isolated Wayland acceptance suite**.

**Accepted progress:** T502 **77% → 78% (+1 pp)**;
category/sound feature inventory **77% → 78%**;
P5 raw **56.75% → 57.00% (+0.25 pp)**;
strict 24-task M0 **90.417% → 90.458% (+0.042 pp)**.
This is reproducible hook-routing compatibility, not
physical speakers/audio-pipeline testing. T503 physical Hyprland
QA remains **51%** and T504 opt-in reversible production
cutover remains **0%**. No installed notification daemon,
live bus or user configuration was touched.

## App-scoped stack tag replacement and quiet category sound (2026-10-25)

Pinned Mako v1.11.0 `dbus/xdg.c:362-371` calls
`get_tagged_notification(state, tag, app_name)` for
`x-dunst-stack-tag` and
`x-canonical-private-synchronous`. A matching tag from
the **same app_name** replaces the previous notification while
retaining its D-Bus ID; the internally created replacement object
still consumes a new allocation ID. A different application
using that same tag must not replace the original notification.
In either case, the new object rematches its category/mode rules
and executes its configured `on-notify` binding
(`dbus/xdg.c:385-430`). Mako's `SetMode` updates the modes
and reapplies existing styles, but does **not** rerun existing
on-notify hooks (`dbus/mako.c:302-350`).

A new isolated real private D-Bus regression checks all these
cases using a temporary output file as a safe fake sound target:
(1) the first tagged success notification runs its category
sound; (2) a same-app tagged warning update runs the warning
sound but preserves the ID; (3) another same-app tagged update
under quiet mode produces **no** sound while preserving the
active notification; (4) exiting quiet mode does not replay the
suppressed sound; (5) a later same-app tagged success update
resumes sound; and (6) a different app with the same tag creates
its own ID and invokes its category sound. Every stage verifies
the exact command output and checks active notification count.
No live audio output, production bus, or installed Mako process
was used. This tests a distinct new-object replacement path from
the previously accepted direct-ID replacement regression.

The first fixture commit had overescaped Rust source string
separators. The corrected fixture was committed before the
accepted run and only the passing corrected result is credited.

- [Pinned Mako source for app-scoped stack tag replacement](https://github.com/emersion/mako/blob/v1.11.0/dbus/xdg.c#L362-L371)
- [Pinned Mako's Notify hook decision and execution](https://github.com/emersion/mako/blob/v1.11.0/dbus/xdg.c#L385-L430)
- [Corrected accepted regression bed938fe](https://github.com/sguzman/posthorn/commit/bed938feca84235efe9898ce2a1f07a62e39fede)
- [Full accepted Rust / private D-Bus / headless Wayland CI 40259507644](https://github.com/sguzman/posthorn/actions/runs/40259507644):
  **347 Rust library tests, 92 private D-Bus tests,
  and the entire isolated Wayland suite passed**.

**Accepted progress:** T502 **78% → 79% (+1 pp)**;
category sound feature inventory **78% → 79%**;
P5 raw **57.00% → 57.25% (+0.25 pp)**;
strict M0 across 24 atomic tasks **90.458% → 90.500%
(+0.042 pp)**. Physical Hyprland acceptance T503 remains
**51%**, and explicit opt-in production cutover T504 remains
**0%**. This is deterministic compatibility evidence, not
physical speaker or real-workstation testing.

## SetModes combinations preserve category sound muting without replay (2026-10-25)

Pinned Mako v1.11.0 `dbus/mako.c:407-442` exposes
`SetModes(as)`. It calls `set_modes` on the supplied
mode array, then `reapply_config` against existing notifications.
Neither operation reexecutes their `on-notify` hooks.
An active mode criterion such as `[mode=quiet]` with
`on-notify=none` can coexist with a visual `[mode=focus]`
background color. A new `Notify` call after the multi-mode
state is updated, however, must select the currently matching
category and mode hooks as usual.

The accepted real private D-Bus regression verifies:
(1) an ordinary category warning executes its file-backed
sound hook; (2) `SetModes(["focus","quiet","focus"])`
deduplicates preserving first order, leaving `["focus","quiet"]`;
(3) a fresh warning notification under both modes preserves the
focus-mode background while suppressing its configured category
sound; (4) `SetModes(["focus"])` cannot retroactively replay
the previous quiet notification sound; (5) a newly posted warning
after quiet ends executes its hook; (6) `SetModes([])`
clears all modes but itself emits no sound; and (7) the next
notification after the empty set still executes its warning hook.
The active in-memory store retains all four notifications.
Every sound is represented by a temporary file-backed shell
command, not by physical speakers or the production daemon.

The first version mistakenly used `VecDeque::to_vec()`
and failed to compile. It was corrected to
`iter().cloned().collect::<Vec<_>>()` before any acceptance
credit. The final commit passed complete CI.

- [Pinned Mako SetModes reapply, without binding replay](https://github.com/emersion/mako/blob/v1.11.0/dbus/mako.c#L302-L350)
- [Pinned SetModes D-Bus method](https://github.com/emersion/mako/blob/v1.11.0/dbus/mako.c#L407-L444)
- [Final accepted regression 38adeb9](https://github.com/sguzman/posthorn/commit/38adeb93b7bee34e3498c4b48af9dbe63dddfffa)
- [Full accepted Rust, private D-Bus, Wayland CI 40260381781](https://github.com/sguzman/posthorn/actions/runs/40260381781):
  **347 Rust library tests, 93 private D-Bus tests and
  complete isolated headless compositor acceptance passed**.

**Accepted progress:** T502 **79% → 80% (+1 pp)**;
category sound feature inventory **79% → 80%**;
P5 raw **57.25% → 57.50% (+0.25 pp)** (display **58%**);
strict M0 **90.500% → 90.542% (+0.042 pp)**
(display **91%**). Real Hyprland T503 remains **51%**;
opt-in production cutover T504 remains **0%**.
This test does not validate actual audio playback or user-device
wake behavior. No live bus, installed Mako or Hyprland config was touched.

## Restored category notification does not replay sound hooks (2026-10-25)

Pinned Mako v1.11.0's `dbus/mako.c:120-141` restores the first
archived notification by reinitializing its style, applying criteria and
reinserting the existing object. It does **not** call
`notification_execute_binding` (`notification.c:381-436`).
Only a subsequent Freedesktop `Notify` can execute the configured
category `on-notify=exec` again.

A private-D-Bus regression
(`tests/private_service.rs::restoring_archived_category_notification_does_not_replay_sound_hook`)
checks the full sequence: the first warning notification runs its file-backed
sound command; quiet mode is entered; the message is dismissed to history
and restored; the category background is preserved but the current quiet
mode overrides `on-notify`; restoration and leaving quiet never replay the
sound; finally an explicit direct-ID warning replacement executes the sound
again while preserving the restored ID.

[Accepted implementation 0a4758a](https://github.com/sguzman/posthorn/commit/0a4758a96fa1d3abd62427b4106e656d7bebfa7e) ·
[successful Rust/private-D-Bus/Wayland CI 40261769648](https://github.com/sguzman/posthorn/actions/runs/40261769648):
**347 Rust library tests, 94 private D-Bus tests, and every isolated
headless Wayland acceptance check passed.**

**T502 80% → 81%; category sound feature 80% → 81%;
P5 57.50% → 57.75% (display 58%);
strict 24-task M0 90.542% → 90.583% (display 91%).**
Physical Hyprland T503 stays 51% and authorized reversible
cutover T504 stays 0%. No speaker, production notification bus,
live Mako daemon or workstation configuration was touched.

## Category sound hooks honor reloaded configuration without replay (2026-10-25)

Pinned Mako v1.11.0's `dbus/mako.c:296-344` rebuilds notification
styles and surfaces during a successful Reload, but does not call
`notification_execute_binding` on existing notes. The next
Freedesktop `Notify` selects the current category and mode rules
and executes the newly configured `on-notify=exec` sound command.

The new private D-Bus regression
(`tests/private_service.rs::reloaded_category_sound_configuration_applies_only_to_new_notifies`)
loads a file-backed warning sound/color configuration, sends a notification,
rewrites the configuration, reloads, confirms live category colors changed
without an extra sound, and confirms the next Notify plays the new sound.
It then enters quiet mode, rewrites and reloads sound/color rules again,
confirms a new notification remains visually categorized and silent,
leaves quiet without replaying any sound, and checks that the following
Notify plays the latest command. All hooks append to a temporary file;
no speaker, installed Mako or public notification bus is involved.

[Accepted regression af546b0](https://github.com/sguzman/posthorn/commit/af546b04c39f753219c9863efd7bc70affb8e96e) ·
[accepted two-job CI 40262257116](https://github.com/sguzman/posthorn/actions/runs/40262257116):
**347 Rust library tests, 95 private D-Bus tests, and the complete
isolated headless Wayland suite passed.**

**T502 81% → 82%; category sound feature 81% → 82%;
P5 57.75% → 58.00% (display 58%);
strict M0 90.583% → 90.625% (display 91%).**
Physical Hyprland acceptance T503 stays 51%, and the explicit-authorization
reversible production cutover T504 stays 0%. Those gates are not credited.

## Rejected Reload cannot partially replace category sound hooks (2026-10-25)

Pinned Mako v1.11.0 `config.c:993-1019` parses a candidate
configuration into an isolated temporary object, and only replaces the
active one after successful parsing. Its `dbus/mako.c:354-368`
returns InvalidConfig on failure without style reapplication. Category
`on-notify=exec` sound hooks and category colors must therefore
retain the last-good configuration after any rejected reload.

The new file-backed private-D-Bus regression
`tests/private_service.rs::invalid_reload_retains_last_good_category_sound_and_quiet_mode`
verifies an initial warning sound; rejects `width=0` alongside
different sound/color settings; retains the former warning color and
command on the next Notify; accepts a subsequent valid new command;
suppresses it under quiet mode; does not replay it when quiet ends;
and executes the new command on the next new notification.
No physical audio or public notification bus is used.

[Implementation commit 7804131](https://github.com/sguzman/posthorn/commit/78041318f6cb3c9473e82ec86ad715de4de60ccb) ·
[accepted two-job CI 40263302366](https://github.com/sguzman/posthorn/actions/runs/40263302366):
**347 Rust library tests, 96 private D-Bus tests and complete isolated
Wayland compositor acceptance passed.**
T502 received **+1 pp** acceptance credit.

## Ordered category and quiet criteria determine sound precedence (2026-10-25)

Pinned Mako v1.11.0 `criteria.c:421-438` applies every matching
criteria section in its configuration-file order. A later
`[category=warning]` section can override an earlier
`[mode=quiet] on-notify=none` rule and run its sound hook even
while quiet mode is active. Putting `[mode=quiet]` last mutes
that same category. Posthorn must preserve this ordered semantics, not
invent an unconditional mute layer.

The private D-Bus regression
`tests/private_service.rs::category_sound_and_quiet_mode_follow_mako_config_section_order`
checks both orders on a live private service across configuration
reloads: category-last runs its file-backed sound while quiet is active;
quiet-last suppresses the next warning while preserving its color;
returning to category-last does not replay old hooks but the next
warning sounds again.

[Implementation commit 44f9740](https://github.com/sguzman/posthorn/commit/44f97402efb4227501b5930cd0c20d14a382c800) ·
[accepted two-job CI 40263555358](https://github.com/sguzman/posthorn/actions/runs/40263555358):
**347 Rust library tests, 97 private D-Bus tests and all isolated
Wayland acceptance checks passed.**
T502 received a further **+1 pp** acceptance credit.

Combined accepted delta: **T502 82% → 84%; category-sound feature
82% → 84%; P5 58.00% → 58.50%; strict 24-task M0
90.625% → 90.708% (+0.083 percentage points).**
T503 physical Hyprland remains at 51%; T504 reversible, explicitly
authorized production cutover remains 0%. Installed Mako, live bus,
desktop configuration and hardware audio remain untouched.

## Category sound expiry and history restoration do not replay hooks (2026-10-25)

Pinned Mako v1.11.0 `dbus/xdg.c:84-91` performs timer expiry by
closing the notification with `MAKO_NOTIFICATION_CLOSE_EXPIRED`.
Only the original `handle_notify` calls
`notification_execute_binding` (`dbus/xdg.c:430`).
The control `RestoreNotification` handler
(`dbus/mako.c:120-141`) reinserts the archived object without
arming another timeout or running its `on-notify=exec` again.

A private D-Bus integration regression
(`tests/private_service.rs::category_sound_expiration_and_history_restore_do_not_replay_hooks`)
verifies the first category sound command, original timer, the
`NotificationClosed(id,Expired=1)` signal, archival to history,
sound-free expiry, quiet-mode restoration preserving category color,
no replacement expiration timer, quiet Notify suppression, no sound on
leaving quiet, and fresh sound only on a subsequent Notify.

[Accepted commit d5b93c0](https://github.com/sguzman/posthorn/commit/d5b93c00459efd5cdff3eb64a6f23911313356f8) ·
[green two-job CI 40265334892](https://github.com/sguzman/posthorn/actions/runs/40265334892):
**347 library tests, 98 private D-Bus tests, complete isolated Wayland suite**.

## App-scoped category sound muting preserves other senders (2026-10-25)

Pinned Mako `criteria.c:421-438` applies each matching selector
in config order. The combination `[app-name=muted category=warning]`
can override a general warning sound without suppressing warning
notifications from other applications. A later `[mode=quiet]` can
temporarily suppress them all.

The private D-Bus regression
(`tests/private_service.rs::app_scoped_category_sound_mute_preserves_other_application_audio`)
sends warnings from two applications, verifies both retain their
category color, confirms only the allowed app runs the sound,
silences both apps in quiet mode without replaying previous hooks,
and verifies app-specific suppression resumes when the ordinary
mode is restored.

[Accepted commit 65ae3ab](https://github.com/sguzman/posthorn/commit/65ae3abb745cc4528e88acfea860a6d9a737d585) ·
[green two-job CI 40265537356](https://github.com/sguzman/posthorn/actions/runs/40265537356):
**347 library tests, 99 private D-Bus tests, complete isolated Wayland suite**.

## A failing category sound command cannot reject Notify (2026-10-25)

Pinned Mako `notification.c:403-435` launches
`on-notify=exec` through a double-fork and does not propagate the
hook shell's exit status to the Freedesktop Notify result
(`dbus/xdg.c:430-434`). Sound-player failure must leave notification
delivery, active history and future mode transitions usable.

The new private D-Bus regression
(`tests/private_service.rs::nonzero_category_sound_command_exit_does_not_reject_notify`)
uses a temporary file-backed sound hook that emits its ID and
exits with status 27. Notify still succeeds and retains the
notification; quiet mode suppresses the failing hook and its
mode changes do not replay it; a later ordinary Notify continues
to execute the hook without losing delivery.

[Accepted commit 228fdd0](https://github.com/sguzman/posthorn/commit/228fdd07bc89c610f4a7ca079f6bed95dab5b3d5) ·
[green two-job CI 40265692081](https://github.com/sguzman/posthorn/actions/runs/40265692081):
**347 library tests, 100 private D-Bus tests, complete isolated Wayland suite**.

Combined accepted delta:
**T502 84% → 87% (+3 pp)**;
category sound parity feature **84% → 87% (+3 pp)**;
P5 **58.50% → 59.25% (+0.75 pp, displays 59%)**;
strict 24-task M0 **90.708% → 90.833% (+0.125 pp, displays 91%)**.
This is file-backed command QA, not physical speaker testing.
T503 physical Hyprland remains **51%**, and T504 explicitly
authorized reversible live Mako cutover remains **0%**.
No production Mako, user notification D-Bus name or desktop
configuration was modified.

## Zero-visible Mako overflow cannot invent popup width (2026-10-26)

Pinned Mako v1.11.0 `render.c:360-361,392-399,424-430,442-484`
computes `max_width` only from *visible real notifications*. The
synthetic hidden-count notification contributes height, but it does not
increase that width. When `max-visible=0`, all real notes are hidden;
the synthetic placeholder cannot establish a drawable canvas by itself.

Posthorn previously gave such a placeholder its own nonzero popup width,
displaying a visual element absent from the pinned Mako canvas geometry.
The fix preserves creation/ID consumption of the hidden pseudo-notification
but returns no drawable frame when the real visible count is zero. The
same invariant also applies at 2x scale.

Implementation and regression:
`src/desktop.rs::all_hidden_notifications_do_not_create_synthetic_only_canvas`,
[cf52987](https://github.com/sguzman/posthorn/commit/cf5298734fc79368adb113f38bbea699d17aa336),
[47b5127](https://github.com/sguzman/posthorn/commit/47b5127e4d7d56f3bf6d42833b141ea109b66c39).
[Both-job successful CI 40324367009](https://github.com/sguzman/posthorn/actions/runs/40324367009)
validates the geometry fix and ID-lifetime regression alongside the full
isolated Sway suite.

The follow-up refactor
[3fe3be8](https://github.com/sguzman/posthorn/commit/3fe3be8d932d2ef8b63c58c92a900c96fad4f336)
centralizes the same zero-visible rule in
`compose_mako_stack_with_visible_count` so callers beyond the
desktop snapshot cannot accidentally invent a synthetic-only canvas.
Its complete Rust, private D-Bus and isolated Wayland CI passed at
[CI 40324626395](https://github.com/sguzman/posthorn/actions/runs/40324626395).
The progress-ledger and roadmap integration commit
[8300a5d](https://github.com/sguzman/posthorn/commit/8300a5dbff33da9671d6b8ab351224e9b457a52f)
passed its own exact-commit two-job
[CI 40325386623](https://github.com/sguzman/posthorn/actions/runs/40325386623):
**348 Rust library tests, 100 private D-Bus tests, and all isolated
Wayland acceptance checks passed**.

Accepted progress credit is for the already green geometry fix:
**T105 layout/style 93% → 94%**;
P1 task average **96.0% → 96.2%**;
strict 24-task M0 **90.833% → 90.875% (+0.042 percentage points)**.
T204 grouping/visibility remains 99%, T402 renderer remains 99%, and
physical Hyprland T503 and authorized production cutover T504
remain 51% and 0% respectively.
No desktop notification bus registration, installed Mako daemon,
Hyprland config or physical workstation state was modified.

## Real Sway confirms hidden-only zero-width and reversible max-visible Reload (2026-10-26)

Pinned Mako v1.11.0 `render.c:360-361,392-399,424-430,442-484`
excludes synthetic hidden-count placeholders from `max_width`.
The prior code correction and its Rust regression ensure that when
`max-visible=0` all sender notifications remain active but
no drawable popup is created. This acceptance extends that specific
source-backed behavior to an actual isolated wlroots compositor.

The guarded 1x private Sway workflow now launches Posthorn on its
development-only D-Bus name, snapshots the headless output as a
baseline, sends a real Freedesktop Notify under `max-visible=0`,
and verifies that a uniquely colored synthetic-only placeholder
does **not** change the screenshot. It then changes **only the
disposable config file** to `max-visible=1`, calls Mako's Reload
control, and confirms that the same previously hidden notification
becomes visible on screen. Reloading back to `max-visible=0`
returns the pixel count to baseline without replacing or
destroying the sender's notification; the fixture finally closes
the notification explicitly.

[Accepted Sway regression b52da32](https://github.com/sguzman/posthorn/commit/b52da32bbc3fc7a814b648feed5b12f1a5d9123a) ·
[green two-job CI 40327188322](https://github.com/sguzman/posthorn/actions/runs/40327188322):
**348 Rust library tests, 100 private D-Bus tests, all isolated
Wayland checks; explicit `Live Sway hidden-only canvas and reversible
max-visible Reload: OK`**. No real Hyprland monitor, production
Mako, speaker or live standard-name notification bus was involved.

Accepted progress: **T105 layout/style 94% → 95%**;
P1 **96.2% → 96.4%**; strict 24-task M0
**90.875% → 90.917% (+0.042 percentage points)**.
T503 physical Hyprland remains 51%; the separately authorized
reversible Mako cutover T504 remains 0%.

## Correction: hidden text clamps to compositor-confirmed surface width (2026-10-26)

**Supersedes the previous entry at this location.** The 7ba8436 /
07ccdc9 implementation and green CI originally earned T105 +1 point
for preserving an unclamped `[hidden]` width. That interpretation
was **incorrect**: upstream Mako v1.11.0 `render.c:113-115` sets
`notif_width = min(style->width, surface->width)` BEFORE constructing
the Pango layout (lines 139-159). This applies to the synthetic
hidden-count notification passed through the same renderer at
`render.c:442-474`. While `render.c:424-430,482` excludes the
hidden placeholder from the requested canvas width, the compositor
confirms that requested surface size and stores it in
`wayland.c:361-379`; subsequent drawing is constrained by the
confirmed width. The old section omitted that essential bound.

[Correction cb0074e](https://github.com/sguzman/posthorn/commit/cb0074e5cf019dec86a34d7ac5a714aae0b59e4d)
restores the source-backed surface-width clamp before Pango. The
negative-control raster test was inverted:
`desktop::tests::wider_hidden_style_is_clamped_to_configured_surface_before_pango`
now requires the narrow, surface-bounded raster including its extra
wrapping and verifies that the unconstrained wide rendering differs.
The configured-surface width is calculated from real notifications,
not the synthetic hidden frame. Actual compositor-forced sizes below
the requested size remain a separate physical/compositor acceptance
gap; no all-display-size parity is implied.

The formerly credited task increment was invalid and is withdrawn:
**T105 96% → 95% (-1 pp), P1 96.6% → 96.4% (-0.2 pp),
strict M0 90.958% → 90.917% (-0.042 pp)**.
This is an evidence correction, not lost working functionality.
The earlier live Sway `max-visible=0 ↔ 1` acceptance remains valid
and is unaffected. Physical Hyprland T503 remains 51% and the
explicitly authorized reversible production cutover T504 remains 0%.

The isolated Wayland suite for the corrected implementation passed
in [CI 40330397284](https://github.com/sguzman/posthorn/actions/runs/40330397284);
the separate Rust job was still at dependency installation at the
time of this source-accounting correction, so complete exact-SHA
CI success is **not claimed** here.

## Private Sway validates compositor-constrained hidden-label Pango height (2026-10-26)

Pinned Mako v1.11.0 `render.c:113-115,139-159` clamps each
notification's width to `surface->width` *before* Pango layout;
that includes the synthetic `[hidden]` notification at
`render.c:442-474`. `wayland.c:361-379` receives the
compositor-configured width, while `render.c:424-430,482`
excludes the hidden pseudo-note from requested surface width.

The new headless Sway fixture tests this with an 84-pixel real
notification and a `[hidden] width=260` rule with an intentionally
long unbroken label. Its private D-Bus service receives two
notifications. The actual compositor snapshot contains an
**84 × 256-pixel** hidden-label region, confirming width-constrained
wrapping rather than unconstrained layout followed by clipping.
The earlier initial fixture failure was a busctl command-line
argument (`-1` interpreted as an option), corrected to a positive
test timeout without changing production behavior.

[Regression c463c02](https://github.com/sguzman/posthorn/commit/c463c02a04ad67e1949e95b777f46482515caf71) ·
[fixture correction dba311b](https://github.com/sguzman/posthorn/commit/dba311b2d4bbd64f4f86620853420f1d564f4b5b) ·
[accepted full two-job CI 40332391193](https://github.com/sguzman/posthorn/actions/runs/40332391193).
**349 Rust library tests, 100 private D-Bus tests and all isolated
Wayland checks passed**, including explicit
`Live Sway hidden text constrained by 84px surface: 84 x 256 OK`.

This is fresh, **correct** acceptance evidence for
**T105 95% → 96% (+1 pp); P1 96.4% → 96.6%;
strict 24-task M0 90.917% → 90.958% (+0.042 pp)**.
This does not reinstate the rejected unclamped-layout reasoning
or imply physical Hyprland/forced-smaller-configure parity.
T503 remains 51%, T504 authorized live Mako cutover remains 0%;
installed Mako, public notification name and real desktop untouched.

## Direct upstream executable differential: hidden Pango geometry exactly matches (2026-10-26)

A new isolated CI workflow builds *two* separate executables:
Posthorn and source-pinned original Mako v1.11.0, whose signed
upstream tag `v1.11.0` points to source commit
`91c6b799855762ad1eb8b4498967c270a339be0c`.
The workflow checks out that exact upstream commit and compiles it
using Meson with icons disabled. Both executables run sequentially
on the **same isolated Sway compositor and dbus-run-session**,
with the identical disposable config. Only Posthorn uses its
development-specific D-Bus name; original Mako uses
`org.freedesktop.Notifications` on that *isolated bus only*.
The first daemon and its layer-shell surfaces are gone before the
second daemon starts. No screenshots are uploaded.

The config deliberately sets real notification `width=84` and
`[hidden] width=260`, `font=Sans 20`, and a long
unbroken label. Both implementations are sent an identical pair
of notifications. The background-color pixel probe measures
the actual Sway-composited hidden label, not an internal raster.

**Accepted measurements:**

| | Posthorn | Original Mako v1.11.0 |
|---|---:|---:|
| x bounds | 1196–1279 | 1196–1279 |
| y bounds | 96–351 | 96–351 |
| width | 84 px | 84 px |
| height | 256 px | 256 px |

The new differential script
`tests/pinned_mako_differential.sh` is guarded to refuse
preexisting Wayland sessions. The accompanying
`.github/workflows/pinned-mako-differential.yml` pins upstream
commit identity and installs its build dependencies inside GitHub
Actions only. Neither installation, test nor source checkout can
alter a local workstation or installed Mako daemon.

[Implementation commit 424c9ee](https://github.com/sguzman/posthorn/commit/424c9ee7fe09fad38fd0118a9fc5d276ec39823d) ·
[successful upstream-executable differential CI 40338017171](https://github.com/sguzman/posthorn/actions/runs/40338017171),
with explicit `Source-pinned Mako/Posthorn hidden-label geometry: MATCH`
and a confirmed **84 × 256-pixel** output on each executable.

The exact-match **external baseline** supplies new acceptance
evidence rather than repeating Posthorn-only tests.
**T105 96% → 97% (+1 pp), P1 96.6% → 96.8% (+0.2 pp),
strict 24-task M0 90.958% → 91.000% (+0.042 pp).**
This narrow differential does not claim all rendering parity,
full compositor-forced size behavior, or physical Hyprland QA.
T503 remains 51% and T504 reversible authorized cutover remains 0%.

## Pinned Mako Pango antialias and markup executable parity (2026-10-26)

A direct source-pinned executable differential exposed a reproducible
text-raster mismatch even when the two implementations produced
identical notification dimensions. Posthorn used Cairo's implicit font
antialiasing whereas original Mako v1.11.0
`render.c:81-96,152-155` explicitly selects
`CAIRO_ANTIALIAS_GRAY` for outputs reporting
`WL_OUTPUT_SUBPIXEL_UNKNOWN` or `NONE`.

The isolated Sway compositor reports that unknown/no-subpixel case.
Posthorn now explicitly selects gray font antialiasing in **both Pango
height measurement and actual painting**, matching the pinned source.
An older independently constructed italic-overhang raster reference
was updated to use the same upstream font options, **without
relaxing its pixel-for-pixel assertion**. The native upstream
configuration selects RGB/BGR subpixel antialias on other physical
outputs, which is **not** claimed as fully verified here.

[Gray option change 13879c0](https://github.com/sguzman/posthorn/commit/13879c0e975f7cab8070448c08ced8f5754f3e7c) ·
[compile correction 4913a56](https://github.com/sguzman/posthorn/commit/4913a568eea47d236052eacefee084c58eccffac) ·
[exact independent italic reference e4b2218](https://github.com/sguzman/posthorn/commit/e4b2218f88993ab12fad3f2fafe16788516ece4e).

The independent executable harness compiles **original Mako v1.11.0
from pinned source commit `91c6b799855762ad1eb8b4498967c270a339be0c`**
and Posthorn, runs each sequentially on the same disposable
Sway and private D-Bus session, and sends identical notifications.
It measures popup geometry and the count of true text-color pixels
for three distinct existing T205 formatting cases:

| Sender/format case | Posthorn text pixels | Original Mako pixels | Canvas |
|---|---:|---:|---|
| Valid bold/italic/underlined body, markup on | 2,318 | 2,318 | 180 × 176 |
| Literal escaped body tags, markup off | 1,420 | 1,420 | 180 × 176 |
| Invalid sender body markup, escaped fallback | 1,619 | 1,619 | 180 × 176 |

The first case **previously differed, 1,961 vs 2,318** under
Cairo's defaults. Explicit grayscale font options remove the
357-pixel discrepancy without widening tolerance.
[Markup-off comparison cf7da93](https://github.com/sguzman/posthorn/commit/cf7da93c6d64b961ef0b90e5349e581abdedf9e4) ·
[malformed markup fallback ee92f04](https://github.com/sguzman/posthorn/commit/ee92f04c595a00ed73bb4b2b58d43a8cfbb41efb) ·
[full upstream differential and Rust/private D-Bus CI 40344072066](https://github.com/sguzman/posthorn/actions/runs/40344072066) ·
[full isolated Wayland and Rust CI 40344072350](https://github.com/sguzman/posthorn/actions/runs/40344072350).
**349 library tests, 100 private D-Bus tests and complete
isolated Wayland acceptance passed.**

Accepted T205 Pango/formatting increment:
**86% → 88% (+2 pp)**; P2 **96.8% → 97.2% (+0.4 pp)**;
strict 24-task M0 **91.000% → 91.083% (+0.083 pp)**.
This does not claim general font equivalence on RGB/BGR
wl_output subpixel geometries, physical Hyprland acceptance
(T503 remains 51%) or the separately authorized reversible
production Mako cutover (T504 remains 0%). No installed Mako,
live standard notification bus or workstation configuration touched.

## Compositor output subpixel options and input-region Pango parity (2026-10-26)

Pinned Mako v1.11.0 `render.c:64-96` reads the **actual
`wl_surface.enter` output**, then sets Cairo font antialiasing to
`GRAY` when the output reports `UNKNOWN` or `NONE`, and to
`SUBPIXEL` with `RGB`, `BGR`, `VRGB`, or `VBGR` order for
the four other `wl_output.geometry` values. If the popup has not
entered any output, Mako leaves Cairo's font options unchanged.
The original implementation in Posthorn always selected grayscale.

Posthorn now tracks `wl_output.geometry` subpixel values, removes
stale geometry on output-global removal, selects antialiasing from the
**compositor-entered surface** rather than the configured output
placement request, and invalidates the frame when this value changes.
Both measured Pango height and painted notification use the same
Cairo setting. The synthetic hidden placeholder uses the same
setting; notification hotspots and click routing also measure with
that exact output-specific setting instead of separately falling back
to grayscale.

Rust regressions cover all five known panel subpixel values, the
unknown value, and the pre-enter/no-output case. An independent
panel-specific hotspot regression checks that every measured input
rectangle has the actual painted popup height and that a click at its
last visible row targets the matching notification.

Implementation: [output geometry and Cairo order](https://github.com/sguzman/posthorn/commit/55547fe0d18883e2b94273b45e461485df71894e),
[compositor-entered output](https://github.com/sguzman/posthorn/commit/1e970848c31432731b5c1482bc6178deb42e273b),
[pre-enter Cairo behavior](https://github.com/sguzman/posthorn/commit/6f7f54b5ea8ea0f90745ee186ed76c7c144a59f4),
[input-region parity](https://github.com/sguzman/posthorn/commit/efa1ca34cfd8b766670abfdb3d36ed6eb6e456a4).
[Exact input-fix Rust CI 40348495332](https://github.com/sguzman/posthorn/actions/runs/40348495332)
passed **352 Rust library tests, 100 private D-Bus tests, and all isolated
Sway/Wayland checks**. Source-pinned executable CI for the same
input-fix commit is tracked independently at
[40348495009](https://github.com/sguzman/posthorn/actions/runs/40348495009);
it must not be described as passed until complete.

**Source-backed partial acceptance:** T205 **88% → 89% (+1 pp)**,
P2 **97.2% → 97.4% (+0.2 pp)**, 24-task M0
**91.083% → 91.125% (+0.042 pp)**. These source-matched and
automated synthetic tests do **not** prove real RGB/BGR hardware
antialias on the user's Hyprland session. T503 physical Hyprland
remains 51%, and T504 reversible production cutover remains 0%.
Installed Mako, the public notification bus, and workstation
configuration remain untouched.

## Actual 2× Pango executable comparison and high-DPI geometry (2026-10-26)

Pinned Mako v1.11.0 `render.c:152-213,318-325` does **not**
simply stretch an existing 1× Pango layout. It sets physical layout
limits to `logical_width * scale` and `logical_height * scale`,
inserts `pango_attr_scale_new(scale)` into the parsed markup
attributes, calculates physical glyph dimensions, and then divides
their measured height by output scale (integer truncation) before
computing the notification's logical popup height.

The previous Posthorn implementation instead premeasured Pango at
1× and enlarged Cairo's context while painting. An independent
pinned-upstream executable comparison on the **same actual scale-2
Sway output** established the defect:

| Before correction, scale 2 | Posthorn | Original Mako |
| --- | ---: | ---: |
| Popup geometry (physical pixels) | 360 × 352 | 360 × 394 |
| Exact text-color pixel count | 13,216 | 13,999 |

Posthorn now uses the upstream scaling attribute and physical Pango
limits in **both** measurement and painting, restores an identity
Cairo matrix for physical font rendering, and uses the configured
maximum height for Pango layout instead of the shrink-wrapped
notification height. The measured physical text height is converted
back to logical height using Mako's integer truncation semantics.

Both live frame presentation and input-region/hit-test geometry use
the same actual output-scale measurement. Unit regressions establish
the source's non-linear font-size behavior at multiple scales rather
than incorrectly requiring a 2× output to have exactly 2× the
1× popup's logical height.

The corrected direct executable comparison is now exact:

| Corrected source-pinned scale-2 fixture | Posthorn | Original Mako |
| --- | ---: | ---: |
| Popup geometry (physical pixels) | **360 × 394** | **360 × 394** |
| Exact text-color pixel count | **13,999** | **13,999** |

[Source-pinned test introduction](https://github.com/sguzman/posthorn/commit/b9aca710d8a6fb728d4f336f90935eeb4059f543) ·
[test harness correction](https://github.com/sguzman/posthorn/commit/f0e5892c495e2a22d9c6195d333bb75266ad41e1) ·
[physical-scale implementation](https://github.com/sguzman/posthorn/commit/97cd33df4ff641f46bcca7c09f483d326e6306e8) ·
[regression fixture correction](https://github.com/sguzman/posthorn/commit/523bf9b17b81ccfc23cf5054cf5126b92899cacf) ·
[final compile fix](https://github.com/sguzman/posthorn/commit/7da6f1f3588a49a06cf5231a326e4de79a51cd12) ·
[successful pinned-Mako comparison 40354143045](https://github.com/sguzman/posthorn/actions/runs/40354143045) ·
[successful Rust/Wayland CI 40354142427](https://github.com/sguzman/posthorn/actions/runs/40354142427).

The exact implementation passed **352 Rust library tests, 100
private D-Bus tests, and the full isolated Sway/Wayland acceptance
suite**, including 2× and 9× output workflows.
**Accepted T205 89% → 91% (+2 pp), P2 97.4% → 97.8% (+0.4 pp),
strict 24-task M0 91.125% → 91.208% (+0.083 pp).**

This proves the observed 2× case and unit-scale geometry, **not**
general font equivalence on physical RGB/BGR panels or the user's
actual Hyprland session. T503 physical Hyprland remains 51%;
T504 explicitly authorized reversible production cutover remains 0%.
Original installed Mako, public notification bus, and workstation
configuration were never modified.

## Pinned Mako rounded-popup Pango height-cap executable parity (2026-10-26)

Original Mako v1.11.0 uses `style->height` to set the Pango
text layout limit in `render.c:139-155` and **only afterward**
expands the painted popup to satisfy the left and right rounded
corner-height pairs in `render.c:208-213`. The physical popup
becoming taller must not give the text a second opportunity to
reflow beyond the configured maximum.

A private test config sets **height=45, width=180, font=Sans 17,
padding=4, border-radius=55**, and sends the *same* synthetic
markup-formatted notification to Posthorn and original pinned
Mako sequentially, on the same isolated scale-1 Sway display.
The 55px upper/lower corner radii force a roughly 110px popup
even though the Pango content is still height-constrained.

| Captured metric | Posthorn | Original Mako |
| --- | ---: | ---: |
| Painted background bounds | **178 × 110 px** | **178 × 110 px** |
| Text-color pixels | **106** | **106** |

The exact glyph-coverage match is important: testing only popup
height would not detect a renderer that incorrectly displays
additional lines of text after enlarging the background.

[Implementation of upstream-configured Pango cap](https://github.com/sguzman/posthorn/commit/97cd33df4ff641f46bcca7c09f483d326e6306e8) ·
[permanent upstream-executable regression](https://github.com/sguzman/posthorn/commit/c9b6b81ec4fb89a596d8418c1d5352e07874f2ad) ·
[rounded generated-roadmap correction](https://github.com/sguzman/posthorn/commit/64cf19d78612eb75d91e470f4c481c720e507521) ·
[green source-pinned differential 40355426136](https://github.com/sguzman/posthorn/actions/runs/40355426136),
which also passed **352 Rust library tests, 100 private D-Bus
tests, and earlier exact 1×/2× executable comparisons**.

**New independent source-backed acceptance:** T105
**97% → 98% (+1 pp)**, phase P1 **96.8% → 97.0% (+0.2 pp)**,
strict 24-task M0 **91.208% → 91.250% (+0.042 pp)**.
This proves the accepted rounded height-cap fixture, not all
possible font/rendering layouts. Physical Hyprland T503 remains
51%; separately authorized reversible production cutover T504
remains 0%. Live Mako, the public notification bus, and workstation
configuration were not changed.

## Physical 2× Pango horizontal text alignment executable parity (2026-10-26)

T205's source-pinned markup and layout acceptance now checks
`text-alignment=center` and `text-alignment=right` on a real
**scale-2 private Sway output**. Unlike a color-pixel count,
the **absolute text-color pixel bounding box** detects incorrectly
positioned ink when the same glyphs were rendered elsewhere.

The fixture starts original Mako v1.11.0 and Posthorn separately,
with the same synthetic notification, fonts, configuration,
private D-Bus, and screenshot conditions, then checks the
painted notification rectangle, exact text-color count, and
actual ink bounds.

| Alignment | Posthorn text-color bounds (x0 x1 y0 y1) | Mako text-color bounds | Text-color pixel count, both |
| --- | --- | --- | ---: |
| Center | **966 1236 16 114** | **966 1236 16 114** | 3,169 |
| Right | **998 1268 16 114** | **998 1268 16 114** | 3,169 |

Both cases show identical full-popup bounds
`920 1279 0 123`. Exact ink-bounds equality also
verifies the 32-physical-pixel horizontal translation from
center to right; total pixels alone would not prove alignment.

[Permanent executable test 289e6d8](https://github.com/sguzman/posthorn/commit/289e6d89ef84dfb88d5b8a1ff5e4e45bb869d3b1) ·
[successful pinned-Mako comparison 40356399096](https://github.com/sguzman/posthorn/actions/runs/40356399096) ·
[successful full Rust/Wayland suite 40356399258](https://github.com/sguzman/posthorn/actions/runs/40356399258):
352 Rust library tests, 100 private D-Bus tests, and all
isolated compositor checks.

**Independent T205 acceptance:** **91% → 92% (+1 pp)**;
P2 **97.8% → 98.0% (+0.2 pp)**;
strict 24-task M0 **91.250% → 91.292% (+0.042 pp)**.
The checks are source-backed and use disposable CI. They
do not establish physical Hyprland T503, nor authorize
production Mako cutover T504. Those gates remain at
51% and 0% respectively. Installed Mako, the standard
user notification bus, and workstation setup remain unchanged.

## Entire decoded 2× screenshot parity for styled, aligned and fallback markup (2026-10-26)

The pinned Mako v1.11.0 `render.c:152-155` and
`notification.c:291-346` require Pango markup parsing, escaped
fallback, explicit disabled markup, and font scaling to agree.
The earlier output-scale correction was accepted on text-color counts
and popup dimensions. The new pinned executable fixture additionally
checks exact placement/ink bounds for center/right text and covers
two previously untested physical-scale formatting branches:
`markup=0` with literal tags, and `markup=1` with a malformed
sender-supplied body.

All test notifications were sent separately to independent binaries
on **the same isolated actual 2× Sway output** and private D-Bus,
using a disposable fixed font/configuration. Original installed Mako
and the desktop session remained untouched.

| 2× formatting case | Posthorn background/text-color pixels | Original Mako | Ink rectangle |
| --- | --- | --- | --- |
| Markup off, escaped literal tags | 360 × 394 / 11,038 | exact match | (931,16)–(1249,378), exact |
| Invalid sender markup, escaped fallback | 360 × 394 / 10,602 | exact match | (931,16)–(1248,374), exact |

A separate PNG probe compares all decoded image pixels with stride
awareness. It reported **zero different pixels in each of five
actual compositor screenshots**, not merely matching count and bounds:
styled markup, center alignment, right alignment, literal-tag
fallback, and malformed-markup fallback.

[Literal and malformed source-pinned tests](https://github.com/sguzman/posthorn/commit/186b8ad8711e6f918fd7425c43e61431ec997b8c) ·
[decoded full-screen PNG comparison](https://github.com/sguzman/posthorn/commit/5ac957cd211db91dba5b648a5d314faefc0b9193) ·
[green exact pinned-executable CI 40357633575](https://github.com/sguzman/posthorn/actions/runs/40357633575).
The latter passed 352 Rust library tests, 100 private D-Bus
tests, and all cross-executable cases. The separate full
Rust/Wayland CI for this exact implementation is
[40357633034](https://github.com/sguzman/posthorn/actions/runs/40357633034)
and must be checked independently before claiming its completion.

**Source-backed acceptance:** T205 **92% → 93% (+1 pp)**,
P2 **98.0% → 98.2% (+0.2 pp)** and strict 24-task MVP
**91.292% → 91.333% (+0.042 pp)**.
The physical Hyprland workstation (T503 51%) and
expressly authorized production cutover (T504 0%) remain
unattempted. General renderer equivalence is not asserted
beyond these observed fixtures.

## Pixel-exact physical 2× Pango wrapping and ellipsized-height parity (2026-10-26)

The pinned v1.11.0 `render.c` uses
`PANGO_WRAP_WORD_CHAR` plus `PANGO_ELLIPSIZE_END`, with
physical `PangoAttrScale` and integer-truncated logical
heights. Matching short markup is insufficient to establish
parity for long mixed-markup notifications and wrapped text.

The permanent differential exercises the **same synthetic
long notification** containing XML-escaped ampersands, Greek
letters, bold and italic spans, and an intentionally long
unspaced word, with `width=154`, `font=Sans 17`,
and two separate configured height caps on the **same
actual scale-2 disposable Sway output**:

| High-DPI source comparison | Posthorn | Mako v1.11.0 | Decoded full screenshot |
| --- | --- | --- | --- |
| `height=62`, ellipsized cap | Bounds `972 1279 0 73`, 1,521 text-color pixels | Identical | **0 mismatched pixels** |
| `height=238`, long wrapped text | Bounds `972 1279 0 451`, 15,638 text-color pixels | Identical | **0 mismatched pixels** |

These captures independently establish that the source's small
height cap truncates output and that the larger cap displays
substantially more wrapped glyph ink. The exact comparison
operates on **decoded pixel channels**, not PNG file bytes.

[Permanent guarded differential 5aa2143](https://github.com/sguzman/posthorn/commit/5aa214370abe2319b74a1e14734a33082d7ef4e2) ·
[green original-Mako differential 40359369008](https://github.com/sguzman/posthorn/actions/runs/40359369008) ·
[green full Rust/isolated Wayland CI 40359369001](https://github.com/sguzman/posthorn/actions/runs/40359369001).
The exact source comparison also passed **352 Rust library
tests, 100 private D-Bus tests**, and all preceding
cross-executable screenshot cases.

**Independent T205 acceptance:** **93% → 94% (+1 pp)**,
P2 **98.2% → 98.4% (+0.2 pp)**,
strict 24-task M0 **91.333% → 91.375% (+0.042 pp)**.
Neither disposable compositor comparison nor its test harness
counts as physical Hyprland T503 (51%), a real RGB/BGR panel
test, or T504 authorized live Mako cutover (0%). The
installed daemon and production D-Bus session are unchanged.

## Exact 2× Pango final-format template fallback (2026-10-26)

The final **formatted template**, not merely an individual
notification body substitution, is validated by Pango before
painting. In pinned Mako v1.11.0, an unmatched opening
`<b>` in `format=<b>Alert: %s` leaves the entire
substituted template visible literally. Valid
`format=<b>Alert: %s</b>` is interpreted as markup.
Posthorn already had an isolated renderer unit test;
the guarded executable differential now confirms **both
branches against original Mako** on a genuine 2× Sway output
using private D-Bus and identical synthetic notifications.

| Format | Both popup bounds | Both text-color counts | Entire decoded image differences |
| --- | --- | ---: | ---: |
| Valid paired bold tags | `896 1279 0 181` | 8,037 | **0** |
| Invalid unmatched opening tag | `896 1279 0 181` | 6,438 | **0** |

The different color counts also establish that the two
Pango branches paint **materially different content**,
while still matching Mako byte-for-byte at the pixel level
after independent compositor capture.

[Fixture `dbbc8f1`](https://github.com/sguzman/posthorn/commit/dbbc8f1a3783a0f66232e6536ade74d797fd0e2c) ·
[green pinned upstream 40361560398](https://github.com/sguzman/posthorn/actions/runs/40361560398) ·
[green Rust and isolated Wayland CI 40361560161](https://github.com/sguzman/posthorn/actions/runs/40361560161).
The pinned comparison passed **352 Rust library tests,
100 private D-Bus tests**, and all prior 2× Pango fixtures.

**Source-backed T205 acceptance:** **94% → 95% (+1 pp)**,
P2 **98.4% → 98.6% (+0.2 pp)**, strict 24-task M0
**91.375% → 91.417% (+0.042 pp)**.
Only these specifically tested 2× Pango cases are
accepted. Physical Hyprland T503 remains 51%, reversible
production Mako cutover T504 remains 0%; neither
production daemon nor real session bus was changed.

## Exact compositor-rendered ordinary and hidden format specifier parity (2026-10-26)

Two independently guarded source-pinned executable tests now
cover the previously unverified rendering branches of
`notification.c:format_notif_text` and
`notification.c:format_hidden_text`. Both use the exact
original Mako v1.11.0 binary and Posthorn on the same
disposable **physical scale-2 Sway compositor** and
private D-Bus. Full decoded compositor screenshots,
not just counts or bounds, must match.

### Ordinary notification fields

`format=%a / id=%i / group=%g / 100%%\\n%s\\n%b`
exercises app name, numeric notification ID, group count,
literal percent, summary and valid sender-body Pango markup
together. Sender XML special characters also pass through
normal markup escaping. Both independently rendered:
- Background bounds `800 1279 0 451`.
- **19,589** exact text-color pixels.
- **Zero decoded differing screenshot pixels**.

[Fixture `a907a85`](https://github.com/sguzman/posthorn/commit/a907a85fc57097d821de51482cf0f4e98151da4d) ·
[successful exact pinned-Mako differential 40366086816](https://github.com/sguzman/posthorn/actions/runs/40366086816).

### Synthetic hidden notification fields

`[hidden] format=Hidden %h of %t` exercises the separate
hidden-count and total-count formatting path. On the same
actual scale-2 compositor, a first hidden plus one visible
notification paints a violet background at
`1000 1279 124 247`, and adding another notification
preserves those bounds but changes the hidden text.

Both **two-notification and three-notification** full
screenshots matched original Mako exactly, **zero decoded
pixel differences**. The test explicitly rejects an
unchanged Posthorn screenshot after the third Notify, so
matching stale frames cannot falsely pass.

The initial harness accidentally left the preceding original
Mako daemon holding the *disposable CI bus name*. This was
corrected without changing the renderer. The following
source comparison exposed a second harness issue:
`capture_bounds()` accepted a stale frame while the
background dimensions stayed constant. The final test
waits for an actually changed screenshot in each daemon
before cross-executable comparison; this version passed.

[Hidden field fixture `39cdb14`](https://github.com/sguzman/posthorn/commit/39cdb14df84057e2f23e608ac5c73d403a6ca385) ·
[CI fixture cleanup `1f20132`](https://github.com/sguzman/posthorn/commit/1f2013216e776c01c8743c2625b16ae33bfadd80) ·
[redraw synchronization `ead7cbb`](https://github.com/sguzman/posthorn/commit/ead7cbb4ccc168848f3b01563528d16558cd0aa4) ·
[green pinned-Mako comparison 40367480557](https://github.com/sguzman/posthorn/actions/runs/40367480557) ·
[green full Rust/Wayland suite 40367480489](https://github.com/sguzman/posthorn/actions/runs/40367480489).

**Two distinct source-backed T205 acceptance increments:**
T205 **95% → 97% (+2 pp)**; phase P2
**98.6% → 99.0% (+0.4 pp)**; strict
24-task M0 **91.417% → 91.500% (+0.083 pp)**.
This acceptance does not prove grouped non-default
`%g` values, every style/compositor combination,
physical Hyprland T503 (51%), or production
Mako cutover T504 (0%). Installed Mako and the
real session's public notification bus are untouched.

## Mutable grouped %g and private desktop CLI Reload exact source parity (2026-10-26)

Two existing MVP gaps have source-pinned cross-executable acceptance
in the **same isolated compositor and private D-Bus session**. The
original Mako v1.11.0 is compiled at the repository's pinned hash;
the user's installed Mako and public notification name are untouched.

### Notification grouped count: first member and transition to two

Mako's `notification.c:group_notifications` only writes the C
`group_count` field after a successful criteria grouping pass,
**including a singleton count of one**. An ungrouped notification
retains a zero field even though Posthorn's internal grouping
projection uses one as the singleton group-length sentinel.

Posthorn now records the **source field-assigned state** independently
of its scheduled grouping/order replay. The flag is initialized false,
set on actual regrouping matches (even one), cleared on grouping reset,
and preserved through explicit-ID replacement. Both popup formatting
and Pango-measured pointer geometry use that source-visible state.

The fixture uses `group-by=app-name`, `format=Group size: %g / %s`,
and an invisible secondary member, forcing the same visible leader to
change from a real one-member group to a two-member group:

| Original Mako/Posthorn 2× group state | Identical full-image bounds | Text-color pixels, both | Decoded difference |
| --- | --- | ---: | ---: |
| First matching notification | `860 1279 0 177` | 6,387 | **0 pixels** |
| Second same-app notification | `860 1279 0 193` | 9,833 | **0 pixels** |

The earlier attempted shortcut that conflated `grouping_applied`
with source group-count initialization failed a replacement
regression. The separated flag fixed both the former ungrouped
markup differential and this grouped source case. **354 Rust
library tests** and **100 private D-Bus tests** passed on the
successful source-pinned comparison.

[Source group metadata correction 38f78d5](https://github.com/sguzman/posthorn/commit/38f78d5f20c29530910cdc9b6d514dea23fbe870).

### Real private Wayland CLI startup and Reload

The explicit development-only presenter now parses Mako-compatible
CLI overrides in original argument order, applies them *after*
the selected config file, and retains them across the real
`fr.emersion.Mako.Reload` D-Bus call. The normal production
`org.freedesktop.Notifications` ownership path remains disabled.

A fixture sets `width=144`, `height=165`, then overrides
those to `--width=205 --height=103`; it next changes the
disk config to `width=300`, `height=180` and a different
background and reloads each daemon independently.
The screenshot comparison waits for **three successive stable
decoded frames** after the changed background is visible.
This caught and eliminated a transient comparison against an
incompletely repainted Mako surface.

| Exact source-pinned scale-2 render | Both full-image bounds | Decoded difference |
| --- | --- | ---: |
| CLI startup | `870 1279 0 123` | **0 pixels** |
| Post-Reload, CLI width retained | `870 1279 0 123` | **0 pixels** |

[CLI-enabled private presenter 7449d1f](https://github.com/sguzman/posthorn/commit/7449d1fe0e90c901de3b81de76814207f2a92904) ·
[Reload acceptance fixture 8f67c60](https://github.com/sguzman/posthorn/commit/8f67c6037b6f5983af11564853e252361ef7cc97) ·
[stable compositor comparison f3eee9c](https://github.com/sguzman/posthorn/commit/f3eee9c39cbaee89837bde083a4466a130b35581) ·
[successful pinned-original-Mako CI 40373410919](https://github.com/sguzman/posthorn/actions/runs/40373410919) ·
[Rust and Wayland CI 40373411172](https://github.com/sguzman/posthorn/actions/runs/40373411172).

**Conservative independent source-backed credit:** T205
**97% → 98% (+1 pp)**; T303 **87% → 88% (+1 pp)**.
P2 **99.0% → 99.2%**, P3 **95.0% → 95.333%**,
strict 24-task M0 **91.500% → 91.583% (+0.083 pp)**.

Remaining user-workstation physical Hyprland T503 is **51%**;
authorized reversible production Mako cutover T504 is **0%**.
These tests establish only the tested isolated source cases.

## Executable Mako config include expansion and nested symlink alias parity (2026-10-26)

T104 now has pinned-Mako **actual compositor executable parity**
in addition to parser unit regressions for include resolution and
stacked styles. The test starts Posthorn and original Mako v1.11.0
independently under one disposable scale-2 Sway compositor and
private D-Bus, never the installed production notification daemon.

Two independently rendered configurations exercise source
`config.c:expand_config_path` / `load_config_file`:

1. A **three-level absolute include chain** loads a leaf through a
   symlink alias, with the middle and root files applying their own
   ordered width overrides. The source and Posthorn both render
   the final **422 physical-pixel** popup `858 1279 0 127`,
   **3,169** text-color pixels and **0 differing decoded
   screenshot pixels**.
2. `include=~//tilde-leaf.conf` expands beneath a **disposable
   HOME**. The root file's width override wins over the leaf,
   yielding the same **414 physical-pixel** popup `866 1279 0 127`,
   **3,169** text-color pixels and **0 differing decoded
   screenshot pixels**.

The fixture verifies each expected width and that meaningful
text is drawn before comparing the entire PNG's decoded pixel
channels, preventing a trivially empty screenshot from passing.
Independent daemon runs never use real HOME or the production
D-Bus name.

[Committed source-pinned fixture 1de6606](https://github.com/sguzman/posthorn/commit/1de6606115e7ac7188c7a0f71bf6ee142b40c666) ·
[passing original Mako differential 40376479281](https://github.com/sguzman/posthorn/actions/runs/40376479281) ·
[passing full Rust and isolated Wayland CI 40376478759](https://github.com/sguzman/posthorn/actions/runs/40376478759).

**Accepted T104 progress:** **94% → 95% (+1 pp)**.
P1 **97.0% → 97.2%**, strict 24-task M0
**91.583% → 91.625% (+0.042 pp)**.
These acceptance cases do not establish all possible include
filesystems, physical Hyprland T503 (51%), or authorized
production cutover T504 (0%); live Mako remains unchanged.

## Nested include live Reload and missing-file rejection executable parity (2026-10-26)

The original Mako v1.11.0 executable and Posthorn both load a
**nested absolute configuration include through a symlink** on
an isolated scale-2 Sway output. Unlike a static parser test,
the same running daemon must re-read a modified included leaf
during the real `fr.emersion.Mako.Reload` method and then
reject a missing leaf **without disrupting the last valid
notification image**.

The private compositor fixture independently establishes:
- Initial included style: both render bounds
  `858 1279 0 127` and **3,169** text-color pixels.
- After the included file's `padding=5` becomes
  `padding=12` and the background changes to `#2A7F9B`,
  both render settled bounds **`858 1279 0 155`**.
  Three identical consecutive decoded frames are required
  before acceptance.
- After deleting the included symlink target, both reject
  `Reload` with the source-compatible `InvalidConfig`
  error while retaining the exact previously settled
  screenshot. The rejection adds **zero differing pixels**.
- The source-pinned cross-executable screenshot comparison
  of both the successful Reload and the rejected Reload
  produces **zero decoded pixel mismatches**.

[Permanent source-pinned fixture `394297c`](https://github.com/sguzman/posthorn/commit/394297c764efbff82792cb9c221b43d9c70cd43c) ·
[green original-Mako executable differential 40377546798](https://github.com/sguzman/posthorn/actions/runs/40377546798) ·
[green full Rust/Wayland CI 40377546761](https://github.com/sguzman/posthorn/actions/runs/40377546761).
The pinned differential also passed **354 Rust library tests
and 100 private D-Bus tests**.

**Two independent MVP acceptance increments:** T104
**95% → 96% (+1 pp)**, T303 **88% → 89% (+1 pp)**.
P1 **97.2% → 97.4%**, P3 **95.333% → 95.667%**
(displayed **96%**), strict 24-task M0 **91.625% →
91.708% (+0.083 pp)**. The experiments exercise isolated
Mako vs Posthorn only. Physical Hyprland T503 stays 51%;
a production Mako replacement under T504 remains 0%
and requires explicit authorization.

## Implicit Mako HOME/XDG config precedence and runtime rediscovery (2026-10-26)

Pinned `config.c:get_default_config_path` chooses the
readable legacy `~/.mako/config` before
`$XDG_CONFIG_HOME/mako/config` when no `-c` is provided.
Pinned `reload_config` calls `parse_config_arguments`
again, repeating that discovery and reapplying CLI options.
The former Posthorn private Wayland and D-Bus entrypoints
required an explicit path, despite the parser already
implementing implicit path lookup.

Posthorn's explicit **private-only** presenter and service now
support startup without a path, retaining an
`implicit-default` runtime flag. Reload rediscovers the
current default file instead of pinning the startup path.
Explicit `-c`/positional file paths remain fixed and
the production notification service remains unavailable.

The guarded executable fixture creates **disposable**
`HOME` and `XDG_CONFIG_HOME` trees, writes both
configs, starts each daemon without `-c`, verifies the
higher-priority legacy style, deletes that file, and
performs a live `fr.emersion.Mako.Reload`. Both original
Mako v1.11.0 and Posthorn then adopt the XDG style
while preserving the source-compatible CLI
`--width=205` override.

| Actual 2× Sway result | Posthorn | Pinned Mako | Pixel differences |
| --- | --- | --- | ---: |
| Default legacy style | `870 1279 0 127`, 3,169 text-color pixels | Identical | **0** |
| XDG style after Reload | `870 1279 0 155` | Identical | **0** |

An additional **subprocess-isolated Rust test** verifies
priority when both files exist, re-selection when the
legacy file disappears, CLI override preservation,
atomic rejection when the XDG file becomes invalid,
and fallback to built-in defaults when neither file
exists. It never mutates HOME or XDG in the parent
test process.

[Implementation `9afe153`](https://github.com/sguzman/posthorn/commit/9afe153691d56394852b03bb43b0fa6af70ae0b3) ·
[source-pinned screenshot fixture `1c70798`](https://github.com/sguzman/posthorn/commit/1c7079885570c57680c335915d2cde079b691aee) ·
[isolated unit regression `9255022`](https://github.com/sguzman/posthorn/commit/9255022490dda86c67981000d33d439ccc78f45b) ·
[green exact pinned-Mako differential 40381602201](https://github.com/sguzman/posthorn/actions/runs/40381602201) ·
[green Rust/Wayland CI 40381602198](https://github.com/sguzman/posthorn/actions/runs/40381602198).

The exact source comparison passed **355 Rust library tests,
100 private D-Bus tests, and every previous guarded
compositor fixture**.

**Independent progress credit:** T104 **96% → 97%
(+1 pp)** and T303 **89% → 90% (+1 pp)**;
P1 **97.4% → 97.6%** (rounded 98%),
P3 **95.667% → 96.000%**,
strict 24-task M0 **91.708% → 91.792%
(+0.083 pp)**.

These results prove only isolated config selection and
Reload equivalence, not physical Hyprland T503 (51%)
or production cutover T504 (0%). Installed Mako,
its public D-Bus name and user HOME are untouched.

## Source-pinned replacement-ID group split and rejoin semantics (2026-10-26)

Pinned `dbus/xdg.c:handle_notify` reuses an existing
notification ID while updating its application name, then
reapplies the notification's group-by criteria without
allocating a new ID. `notification.c:group_notifications`
assigns group indices and counts after every matching pass.
Posthorn's store already preserved explicit replacement
identity and group metadata, but the full compositor sequence
had not been compared against original Mako.

A permanent guarded fixture configures `group-by=app-name`,
`format=Group %g / %s`, an invisible secondary member, and
`max-visible=1`. On one **isolated scale-2 Sway compositor**
and private D-Bus session, both daemons independently execute:

1. Two different IDs arrive from the same app and form a group.
2. The secondary ID is replaced by a notification from a
   different app, leaving its original group without changing ID.
3. That same ID is replaced again with its original app name,
   rejoining the former group.

The fixture requires all three states to become stable *and*
requires the split and rejoin to change the displayed image.
Each **entire decoded screenshot** matched the corresponding
original Mako screenshot with **zero differing pixels**.

[Permanent executable comparison `9ca40eb`](https://github.com/sguzman/posthorn/commit/9ca40ebbd7762eba98ccfe7215db66c47833caea) ·
[green pinned-Mako differential 40383427830](https://github.com/sguzman/posthorn/actions/runs/40383427830) ·
[green full Rust/isolated Wayland CI 40383427898](https://github.com/sguzman/posthorn/actions/runs/40383427898).

**Conservative T103 source grouping acceptance: 94% → 95%
(+1 pp); P1 97.6% → 97.8% (+0.2 pp); strict
24-task M0 91.792% → 91.833% (+0.042 pp).**
T204 remains 99% because this fixture does not establish
universal group replacement/close/sort behavior. Physical
Hyprland T503 stays 51%, authorized production cutover
T504 stays 0%. Installed Mako and its public bus
were never contacted.

## Implicit HOME/XDG candidates disappear: source-pinned built-in Reload recovery (2026-10-26)

Pinned Mako v1.11.0 `config.c:get_default_config_path` and `reload_config` re-evaluate implicit configuration selection on every Reload. Posthorn now has an **executable scale-2 compositor comparison** proving the complete transition **legacy `~/.mako/config` → `$XDG_CONFIG_HOME/mako/config` → built-in Mako defaults**. Both test daemons run on the same isolated Sway output and distinct disposable D-Bus services. HOME and XDG trees are temporary, restored between binary runs; no real user files or production bus were accessed.

The third transition removes the last candidate file and issues `fr.emersion.Mako.Reload` while the notification remains active. It waits for stable background pixels and compares decoded full screenshots, not merely geometry. Both daemons painted the default `#285577` background at physical bounds **854 1255 24 171**; legacy, XDG, and built-in screenshots each had **zero differing pixels**. The existing `--width=205` CLI override remains in use. The fixture's first iteration exposed an **isolation mistake** (XDG fixture was not restored before the second binary), corrected before acceptance; the red intermediate run is not source-parity evidence.

The same verified pinned-Mako workflow additionally compares grouped notification explicit-ID replacement, group splitting and rejoining, with pixel-exact equality at every stage. This is bounded T204 regression evidence, **not** a claim that all grouping, physical Hyprland, or T204's remaining final acceptance gates are complete.

[Built-in fallback fixture 6370ac2](https://github.com/sguzman/posthorn/commit/6370ac2b9aee74470fd2c5bf6b74ec7e48035438) · [isolated fixture restoration e7c9cf0](https://github.com/sguzman/posthorn/commit/e7c9cf0fbdf22ffd5e331d516a6fb62cad9d7818) · [grouped replacement test 9ca40eb](https://github.com/sguzman/posthorn/commit/9ca40ebbd7762eba98ccfe7215db66c47833caea) · [pinned differential 40383427830](https://github.com/sguzman/posthorn/actions/runs/40383427830) · [Rust/Wayland 40383427898](https://github.com/sguzman/posthorn/actions/runs/40383427898). **355 Rust library tests, 100 private D-Bus tests, full headless Sway, and original Mako executable comparisons passed** on the same `9ca40eb` commit.

Accepted atomic progress: **T104 97% → 98%, T303 90% → 91%**, strict 24-task M0 **91.833% → 91.917% (+0.083 percentage points)**. P1 **97.8% → 98.0%**, P3 **96.000% → 96.333%**. These hosted tests do not count toward physical Hyprland T503 (still 51%) or the separately authorized reversible live Mako switch T504 (still 0%).

## Reappearing implicit HOME config and atomic rejected Reload parity (2026-10-26)

Pinned Mako v1.11.0 `config.c:get_default_config_path` searches the legacy
`~/.mako/config` before the XDG candidate on every config parse;
`reload_config` creates a fresh temporary config and keeps the old
one on parsing failure. Posthorn's private daemon follows these
same transitions when no explicit `-c` was passed.

The source-pinned executable test now exercises a four-stage
lifecycle under a disposable `HOME`/`XDG_CONFIG_HOME`, isolated
scale-2 Sway compositor, and separate test D-Bus name for each binary:

1. Startup selects the legacy candidate; removing it and issuing
   `fr.emersion.Mako.Reload` switches the active popup to XDG.
2. Removing XDG too and reloading selects Mako built-in defaults.
3. Recreating the higher-priority legacy file with invalid
   `width=invalid` **rejects** Reload, leaves the private service
   responsive and retains an unchanged built-in screenshot.
4. Recreating the original valid legacy file and reloading returns
   the existing notification to the **exact original** raster.

The original Mako binary and Posthorn independently passed every
stage. Both invalid-Reload retention screenshots and restored
legacy screenshots matched their own earlier expected frames
with **zero decoded pixel differences**. Corresponding screenshots
across Mako and Posthorn also had **zero differences**.
The built-in fallback retained physical background bounds
`854 1255 24 171`, and the existing `--width=205` override
remained applied after each valid rediscovery.

[Source-pinned reappearance fixture 1fc6497](https://github.com/sguzman/posthorn/commit/1fc649765b977f99243745af463d5b72574f60b0) ·
[Invalid-candidate atomicity regression 1eb991f](https://github.com/sguzman/posthorn/commit/1eb991f6115d174ed018a521ce8c60d4290beffa) ·
[Corrected invalid fixture ec68e3e](https://github.com/sguzman/posthorn/commit/ec68e3e08acbf72c8d8158d138559a86170df071) ·
[Green exact pinned-Mako differential 40386242117](https://github.com/sguzman/posthorn/actions/runs/40386242117) ·
[Green Rust/Wayland CI 40386242051](https://github.com/sguzman/posthorn/actions/runs/40386242051).

**Accepted T303 91% → 92% (+1 pp); P3 96.333% →
96.667% (rounded 97%); strict 24-task M0 91.917% →
91.958% (+0.042 percentage points).** T104 remains 98%;
these are bounded Reload acceptance cases, not proof of
universal file/path behavior. Physical Hyprland T503
remains 51%, production cutover T504 remains 0%. Neither
installed Mako nor the user's real session was touched.

## Explicit -c path remains pinned across unrelated HOME default changes (2026-10-26)

Pinned Mako v1.11.0 `config.c:parse_config_arguments` selects an
explicit `-c` file before implicit default discovery and repeats
that argument selection during `reload_config`. Posthorn's private
desktop must therefore keep the explicit path fixed on Reload,
even when `~/.mako/config` changes, while still rediscovering the
latest contents of its selected `-c` file.

The guarded executable test uses synthetic disposable HOME/XDG,
one isolated actual-scale-2 Sway compositor and a private D-Bus
session. Posthorn and original Mako independently:

1. Start with `-c` pointing to a separate config whose painted
   background is `#705098`, plus `--width=205`. Both show
   physical popup bounds `870 1279 0 127` (410 px wide).
2. Change the implicit legacy HOME file and call
   `fr.emersion.Mako.Reload`. The explicit-path popup
   remains **byte-for-byte identical** to its initial raster.
3. Modify only the explicitly selected config, call Reload
   again, and verify that its `#2A7F9B` replacement color
   appears while its overridden 410 px width is retained.

Both implementations passed every transition, and full decoded
screenshots of the initial, unaffected and changed states had
**zero differing pixels** in their independent source comparison.

[Executable regression 99369a0](https://github.com/sguzman/posthorn/commit/99369a0736daf01458fb36a212cce46660eac38a) ·
[Green pinned-Mako differential 40387586952](https://github.com/sguzman/posthorn/actions/runs/40387586952) ·
[Green Rust/isolated Wayland 40387586986](https://github.com/sguzman/posthorn/actions/runs/40387586986).

**Accepted T303 92% → 93% (+1 pp), P3
96.667% → 97.000%, strict 24-task M0
91.958% → 92.000% (+0.042 percentage points).**
T104 remains 98%, physical Hyprland T503
51%, and authorized production switch T504 0%.
No active Mako, workstation configuration or
production D-Bus ownership was changed.

## Source-pinned file-backed category sounds and quiet mode (2026-10-26)

Mako v1.11.0 implements application-specific and category-specific
sound commands through ordinary ordered criteria with
`on-notify=exec`. The source's `notification_execute_binding`
passes the notification ID as a shell `id` variable; there
is no Mako-native audio protocol. The control interface
`fr.emersion.Mako.SetMode` updates active mode criteria.

A permanent guarded differential now checks the **original
Mako executable** against Posthorn in the same isolated
2× Sway/private D-Bus environment. Both daemons load
synthetic rules for `[category=success]`,
`[category=failure]`, `[category=warning]`, followed
by `[mode=quiet] on-notify=none`. Rather than
playing any sound, the selected hook appends its
category and D-Bus-assigned ID to a disposable file.

Each daemon independently passed this observable sequence:

1. Three notifications with distinct category hints caused
   exactly three matching file-backed hooks.
2. `SetMode quiet` and another warning notification
   produced **no additional hook**, without blocking Notify.
3. `SetMode default` and a new warning notification
   resumed the warning hook exactly once.
4. The complete source-Mako and Posthorn event files
   compared byte-for-byte without differences.

[Executable comparison bcbf1be](https://github.com/sguzman/posthorn/commit/bcbf1bec67ea6729b20c0003681b14e3398c3732) ·
[Green pinned original-Mako differential 40388975526](https://github.com/sguzman/posthorn/actions/runs/40388975526) ·
[Green Rust/Wayland CI 40388975466](https://github.com/sguzman/posthorn/actions/runs/40388975466).
The source-pinned workflow passed 355 library tests,
100 isolated private D-Bus tests and all existing
compositor comparison fixtures.

**Accepted T502 87% → 88% (+1 pp), color/sound feature
87% → 88%, P5 59.25% → 59.50% (rounded 60%),
strict 24-task M0 92.000% → 92.042%
(+0.042 percentage points).** These are file-backed
source semantics, **not physical audio acceptance**.
Real Hyprland T503 remains 51%, and the explicitly
authorized production Mako switch T504 remains 0%.

## Executable category hook ordering, failed command, and color/sound parity (2026-10-26)

The pinned Mako v1.11.0 source applies notification criteria in file
order and launches `on-notify=exec` asynchronously; an external sound
command's nonzero exit code does not reject Notify. Posthorn's
private service now has **three separately observable executable
comparisons** against the exact original Mako binary. All fixtures use
temporary files, isolated Sway at physical scale 2, and private D-Bus.
They never play audio or contact the user's installed daemon.

**Ordered quiet/category criteria during Reload.** Initial
`[mode=quiet]` before `[category=warning]` allows the later
category hook to execute even in quiet. Reload changes section order
to put quiet last, and subsequent warning notifications remain silent.
SetMode default resumes warning hooks. Both implementations produce
identical ordered, file-backed hook events without replay on Reload.
[Fixture 70e98c6](https://github.com/sguzman/posthorn/commit/70e98c63556e29f4ae7201bd0107a510fc6a24c6).

**Failed category hook recovery.** A file-backed category hook
deliberately terminates with exit status 27 after writing its ID.
Both binaries continue to accept notifications, execute subsequent
hooks, suppress them during quiet and resume after quiet. Their
complete event outputs match byte-for-byte.
[Fixture 16feca0](https://github.com/sguzman/posthorn/commit/16feca0d642a22708a51fc91cb8dbbac9c62c0e1).

**Color and hook selection through explicit-ID replacement.**
A success notification paints `#305747` and runs a success hook;
replacing the same ID with warning paints `#715536` and runs its
warning hook. A success replacement in quiet paints green without
a hook, and a warning replacement after quiet resumes the warning
hook. All four full decoded screenshots match source Mako with
**zero differing pixels**, as do the complete file-backed
sound-command event files.
[Fixture 80b3a21](https://github.com/sguzman/posthorn/commit/80b3a21a16fcc993551e70c46154b7e54b67f40a).

[Green pinned-Mako differential 40390943775](https://github.com/sguzman/posthorn/actions/runs/40390943775) ·
[green full Rust/isolated Wayland 40390943760](https://github.com/sguzman/posthorn/actions/runs/40390943760).
The differential passed 355 Rust library tests, 100 isolated private
D-Bus tests and all retained compositor comparisons.

**Conservative accepted progress:** T502 **88% → 91% (+3 pp)**,
category-color/sound feature **88% → 91% (+3 pp)**,
P5 **59.50% → 60.25%**, strict 24-task M0
**92.042% → 92.167% (+0.125 percentage points)**.
These are independent pinned-source parity cases, not physical
speaker/Hyprland acceptance. T503 remains 51%; authorized
production Mako cutover T504 remains 0%. No installed service,
public bus, session config or user's real audio hardware was used.
