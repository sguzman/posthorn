# Mako source baseline and port policy

## Pinned upstream

- Upstream: https://github.com/emersion/mako
- Baseline: **v1.11.0**, published 2026-03-26.
- Later upstream check: `master` commit `cce37cd3405f64eee1b07d4656b5ecbbc912b46d` (2026-06-30). Its restored trailing bottom margin is now reflected in Posthorn's signed popup geometry; the other post-1.11 changes are README/CI changes. This does not establish complete Mako behavior parity.
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

Mako v1.11 `icon.c:resolve_icon` searches `root/*/*/name.*`. It first
tries to parse the icon's nominal size from the first wildcard path
component and uses the second component only when the first is
unsized. Importantly, Mako reads the `@scale` suffix from that same
size-bearing component, not from whichever component happens to
contain an `@`.

Posthorn now follows this ordering, including signed C `strtol` decimal
prefixes and the resulting 64-bit to 32-bit narrowing for nominal sizes.
A nonzero negative size in the first component prevents fallback to the
second; an invalid or zero size permits it, as in Mako. Tests cover this
distinction, overflow, both directory layouts, and ensure that
`64x64/apps@2` does not incorrectly outrank `64x64@2/apps` at 2×.
A Cairo-renderer test also paints differently colored PNGs to verify the
actual selected pixels at 1× and 2× without changing popup geometry.
Rust, D-Bus integration and headless Wayland CI passed at
[commit 788b090](https://github.com/sguzman/posthorn/actions/runs/39981803700).
These acceptance cases do not prove complete monitor-hotplug or live
Hyprland parity.

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

Pinned Mako v1.11 `criteria.c:criteria_matches` compares an
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

The notification scheduler independently used to construct selector traits
from the configured placement string. It now uses the observed output as
well. A lifecycle regression verifies that output-scoped
`default-timeout=210` and `history=0` **do not** match before
compositor assignment, then do match once the surface is observed on
`DP-2`. Both CI jobs passed at
[commit dc26d78](https://github.com/sguzman/posthorn/actions/runs/40006884032).
A stronger 1× compositor acceptance sends a notification with a
sender timeout of -1 and checks that `[output=HEADLESS-1]` selects
the scoped short default timeout, paints its distinctive color, removes
the popup on expiry, and excludes it from `ListHistory` when
`history=0`. The signed D-Bus fixture uses an explicit option terminator
so `busctl` passes -1 as the actual method argument. Rust/private D-Bus
and all headless Sway output scales passed at
[commit a483bbd](https://github.com/sguzman/posthorn/actions/runs/40008223463).

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
