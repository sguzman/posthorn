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
| `config.c`, `criteria.c`, `types.c` | Config parser, criteria, styles, types | Partial; format/group spec validation and sort handling strengthened |
| `notification.c` | IDs, replace, close, timeout, history, groups, format | Timers and dismiss/restore synchronization tested, sorting preserved across restores, grouping/visibility partial |
| `dbus/xdg.c`, `dbus/mako.c`, `dbus/dbus.c` | Desktop interface, Mako control interface | Active on private development bus with expiration signals and property invalidations; system notification name not claimed; controls incomplete |
| `wayland.c`, `surface.c`, `pool-buffer.c` | Wayland, layer-shell, compositor surfaces | Private desktop has continuously updated stacked SHM layer-shell surface, socket polling, pointer and touch events; output/scale QA pending |
| `render.c`, `icon.c`, `cairo-pixbuf.c` | Pango/Cairo rendering, layout and icons | Cairo/Pango markup/ellipsize and colored backgrounds, theme/file icons decoded and drawn; exact automatic heights, image-data hints and HiDPI incomplete |
| `event-loop.c`, `main.c`, `mode.c`, `makoctl.c` | Runtime, modes, timers, control CLI | Private live/headless services, configured hooks, mode state, partial posthornctl including menu; exact text output and reload incomplete |

## Explicit nonclaims

Current Rust code registers only the **private development D-Bus name**;
it is not the default desktop notification service. The private desktop path
renders and updates stacked Wayland popups, with basic mouse/touch bindings,
but has not been visually exercised under Hyprland. Exact Mako auto-height,
icons from D-Bus image-data, grouping placeholders, per-output scaling,
XDG activation, touch QA, configuration reload, and control formatting remain.
Current parsing is a subset; POSIX vs Rust regex and many style options
still need parity tests. The
presence of a config key in a map does not mean the visual behavior is ported.
No MVP/release or workstation cutover until the end-to-end acceptance suite
passes. The README and progress report must not call this a working Mako port.
