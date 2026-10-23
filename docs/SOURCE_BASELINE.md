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

## Known action dictionary limitation

Mako 1.11 writes action entries in internal linked-list order, including repeated
keys, into its `a{ss}` control D-Bus field. Posthorn now preserves those
entries in `ListNotifications` and `ListHistory` through a custom ordered
serializer. Its `posthornctl` client uses a matching ordered deserializer for
menu selection, human listing, and JSON listing. The private D-Bus roundtrip,
binary encoding, and menu selection have passed
[CI acceptance](https://github.com/sguzman/posthorn/actions/runs/39866232367).
The `Notifications` property still passes through zvariant's
`Value`/BTreeMap conversion; this sorts action keys and drops duplicate
entries. D-Bus dictionaries conventionally require unique keys, although
Mako itself emits duplicates. Property wire fidelity therefore remains an
explicit MVP gap; neither this commit nor passing CI establishes full parity.

## Sender-ordered dismissal options

Mako 1.11 parses `DismissNotifications`' `a{sv}` arguments in wire order.
An earlier malformed known option must cause rejection even when a later
duplicate supplies a valid value; last valid occurrences otherwise win.
Posthorn now deserializes arrival-ordered entries and validates each before
constructing the internal options map. Both the parser unit test and the
[private D-Bus duplicate-key regression](https://github.com/sguzman/posthorn/actions/runs/39896396119)
passed Rust and headless Wayland CI at commit `e362363`. This fixes
dismissal input parity, **not** the separate lossy `Notifications` property.

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

Mako v1.11 \`types.c:parse_color\` requires a \`#\` prefix and exactly
six or eight following bytes, but parses those bytes with C
\`strtoul(..., base=16)\`, then truncates to uint32 and appends opaque
alpha for six-byte inputs. Posthorn now reproduces the allowed C-locale
leading whitespace, unary signs, 0x prefixes, and signed wraparound
instead of requiring each byte to be a hex digit. Tests compare known
C fixture values, config acceptance, and exact Cairo background pixels.

Mako's \`config.c:apply_config_option\` recursively loads included files
without a fixed nesting limit. Posthorn no longer rejects the 17th
nested include: 22-file chains are exercised through \`Config::from_path\`
and real private D-Bus Reload. Malformed leaf configurations and
include cycles reject the reload transactionally, retaining the last
good configuration. Canonical include-cycle detection is a safety
divergence from Mako's unbounded recursion, not proof of complete
configuration parity.

The regression suite passed both CI jobs at
[commit 3f3754a](https://github.com/sguzman/posthorn/actions/runs/39950069729).
These findings do not close remaining POSIX grammar, properties, or
real Hyprland parity gates.

## Explicit nonclaims

Normal development commands register only the **private development D-Bus name**; standard Freedesktop registration is exercised solely in an isolated CI session.
It is not the workstation's default desktop notification service. The private desktop path
renders and updates stacked Wayland popups, with basic mouse/touch bindings,
with layer-shell configure/SHM attachment and success, failure, and warning screenshot colors, newest-first stacking, right-anchor placement, post-CloseNotification popup removal, timed expiry/repaint under isolated 1x/2x headless Sway, and dynamically added/removed named output recovery under isolated 1x Sway; it has not been visually exercised under Hyprland. Exact Mako Pango metrics,
replacement-time grouping edge cases, live Hyprland monitor hotplug, fractional-scale behavior, compositor-driven XDG activation and touch QA,
complete mode/reload regrouping semantics and control formatting remain. Existing active timeout deadlines are preserved across reloads, as in the pinned Mako source.
Current parsing is a subset; remaining POSIX vs Rust regex differences and many style options
still need parity tests. The
presence of a config key in a map does not mean the visual behavior is ported.
No MVP/release or workstation cutover until the end-to-end acceptance suite
passes. The README and progress report must not call this a working Mako port.
