# Mako source baseline and port policy

## Pinned upstream

- Upstream: https://github.com/emersion/mako
- Baseline: **v1.11.0**, published 2026-03-26.
- Later upstream check: `master` commit `cce37cd3405f64eee1b07d4656b5ecbbc912b46d` (2026-06-30). Posthorn must audit changes after v1.11.0, including that commit's bottom-margin fix, before declaring parity.
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
| `config.c`, `criteria.c`, `types.c` | Config parser, criteria, styles, types | Partial; format/group validation, progress-color operator syntax, output-matched timer/history criteria, source-backed transactional reload and sorting settings |
| `notification.c` | IDs, replace, close, timeout, history, groups, format | Timers, dismiss/restore and per-notification history policy; indexed grouped visibility (including explicit secondary-member overrides), per-criteria limits, count formatting and per-notification overflow placeholders tested; dynamic grouping remains partial |
| `dbus/xdg.c`, `dbus/mako.c`, `dbus/dbus.c` | Desktop interface, Mako control interface | Private development bus with image-data, image-path, progress, reload, conditional root-style GetCapabilities, ActivationToken ordering and on-notify action/dismiss signal tests; full protocol parity pending |
| `wayland.c`, `surface.c`, `pool-buffer.c` | Wayland, layer-shell, compositor surfaces | Private desktop has stacked SHM layer-shell surfaces with redraw reuse and release-aware SHM buffers, named outputs and integer buffer-scale changes, XDG activation token requests, pointer/touch, anchors, and outer-margin placement; live compositor QA pending |
| `render.c`, `icon.c`, `cairo-pixbuf.c` | Pango/Cairo rendering, layout and icons | Cairo/Pango markup/colors/file and inline icons, progress fill, measured heights, group counts, and signed-margin clipping and ARGB source-over overlap tests; bounded nearest-neighbor HiDPI buffers implemented, native-scaled Pango incomplete |
| `event-loop.c`, `main.c`, `mode.c`, `makoctl.c` | Runtime, modes, timers, control CLI | Private live/headless services, post-grouping on-notify hooks and quiet-mode sound tests, Mako-style `mode -s` with ordered duplicate suppression, text history/list, transactional reload, actions=0 suppression; XDG token protocol covered headlessly, exact control parity incomplete |

## Explicit nonclaims

Normal development commands register only the **private development D-Bus name**; standard Freedesktop registration is exercised solely in an isolated CI session.
It is not the workstation's default desktop notification service. The private desktop path
renders and updates stacked Wayland popups, with basic mouse/touch bindings,
with live layer-shell configure/SHM attachment and success-category screenshot pixels verified under isolated headless Sway; it has not been visually exercised under Hyprland. Exact Mako Pango metrics,
advanced grouping, output hotplug, native-resolution HiDPI text, compositor-driven XDG activation and touch QA,
exact reload semantics for existing active timeouts, and control formatting remain.
Current parsing is a subset; POSIX vs Rust regex and many style options
still need parity tests. The
presence of a config key in a map does not mean the visual behavior is ported.
No MVP/release or workstation cutover until the end-to-end acceptance suite
passes. The README and progress report must not call this a working Mako port.
