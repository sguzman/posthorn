# Posthorn 📯

**A Rust reimplementation of the Mako notification daemon.**

![Posthorn project color, warm amber-orange #E5923C](assets/posthorn-color.svg)

**Project color:** `#E5923C` · CSS `oklch(73% 0.14 63)` — [OKLab color decision](docs/COLOR.md).

Posthorn is being built as a **strict port of modern Mako first**. The MVP
inauguration requires matching Mako's real notification behavior, configuration,
D-Bus APIs, Wayland rendering, actions, history, and controls. Only **after**
that milestone will we add TOML, a graphical notification center with history
and Clear All, channels, SQLite persistence, and a general local message broker.

> **Not yet a working daemon.** Rust now includes configuration/criteria,
> notification lifecycle, timers, sorting/grouping projections, D-Bus
> adapters on an isolated development bus, a Wayland layer-shell preview,
> Cairo/Pango rasterized text/colors, configured on-notify shell hooks, and a
> partial posthornctl CLI. **It does not claim the desktop notifications D-Bus
> name or continuously render notifications.** Icons, compositor integration,
> exact control parity, and live Hyprland QA remain.
> **Keep using Mako.** Do not replace, disable or uninstall it for this build.

## Progress

See [milestones, phases, tasks, features and percentage bars](docs/ROADMAP.md),
all calculated from the machine-readable [progress ledger](docs/progress.json).
The [GitHub milestone and phase work queue](docs/ISSUES.md) links all 12
tracking issues. Progress reports for this repository should include percentage bars for every
milestone, phase, task, and feature touched by that report; avoid presenting
unverified work as complete.

## Source and fidelity

- Baseline: [Mako 1.11.0](https://github.com/emersion/mako/releases/tag/v1.11.0), MIT-licensed.
- Audit later upstream changes against the source baseline before inauguration.
- [Mako-to-Posthorn component map and MVP contract](docs/SOURCE_BASELINE.md).
- No private workstation paths, venting, credentials, or user-specific content
  should ever enter this public repository.

## Development (scaffold stage)

```sh
cargo test --all-targets
cargo run -- --check-config /path/to/mako/config
# In a temporary D-Bus session (never claims the real Mako bus name):
dbus-run-session -- cargo test --test dbus_wire --features dbus-integration
```

`--check-config` is a **development-only** parser command. Success means the
parser recognized the file; it does *not* mean the current Posthorn renderer
would reproduce every setting. Current features remain incomplete until
integration tests demonstrate behavior.

## Headless private D-Bus integration

The opt-in development command `posthorn --experimental-private-service PATH`
starts a headless daemon under **`org.posthorn.Development`** (never Mako's
`org.freedesktop.Notifications` bus name). It uses the exact Freedesktop and
Mako object paths/interfaces for explicit, private-name testing and can
expire/archive notifications with D-Bus close signals. It does **not**
render popups, support ordinary `notify-send` destination routing, or replace
the active Mako daemon. Do not install or autostart this experimental service.

The separate `posthorn --experimental-popup-preview PATH` command opens a
single temporary, synthetic category-colored Wayland popup using the supplied
Mako-format configuration. This is **opt-in developer QA**, not the running
notification daemon. It doesn't claim the desktop D-Bus name or affect Mako.

The experimental `posthornctl` binary exercises Mako-style commands only
against `org.posthorn.Development`. It cannot control or replace your active
Mako daemon. Its `menu` command, production routing, and exact non-JSON
formatting are not yet implemented.

The extra `posthorn --experimental-private-desktop PATH` command is a
**development-only live preview**: it runs the private-name D-Bus daemon
alongside a separate Wayland renderer thread and updates a persistent stacked
popup surface whenever the notification store changes. It does not claim the
real desktop notification D-Bus name, handle clicks or outputs correctly yet,
or replace Mako. Do not autostart it.

## License

MIT. Mako's original authors are credited in [the source baseline](docs/SOURCE_BASELINE.md).
