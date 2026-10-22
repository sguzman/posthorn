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
> adapters on an isolated development bus, a continuously refreshed
> Wayland layer-shell stack with release-aware SHM buffers and surface reuse,
> named output routing with integer-scaled buffers, distinct outer/window and
> internal popup margins, shrink-wrapped Cairo/Pango text and icons, configured
> post-grouping on-notify hooks, basic mouse/touch bindings, XDG activation
> tokens, actions=0 suppression, and category-aware history,
> grouped count formatting, overrideable secondary-member visibility, accurate overflow placeholders, and partial posthornctl.
> **It does not claim the desktop notifications D-Bus name.** Inline
> Freedesktop image-data hints, progress fills and transactional config reload
> are implemented in the private development daemon. Exact Pango layout,
> native-resolution HiDPI text, output hotplug, full control parity, and live
> Hyprland/XDG activation QA remain.
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
the active Mako daemon. The `Reload` control method reads the specified
file again, commits valid changes atomically and leaves invalid reloads
unchanged. Do not install or autostart this experimental service.

The separate `posthorn --experimental-popup-preview PATH` command opens a
single temporary, synthetic category-colored Wayland popup using the supplied
Mako-format configuration. This is **opt-in developer QA**, not the running
notification daemon. It doesn't claim the desktop D-Bus name or affect Mako.

The experimental `posthornctl` binary exercises Mako-style commands only
against `org.posthorn.Development`. It cannot control or replace your active
Mako daemon. Its `menu` command is implemented for private-bus action selection.
Mako's `mode -s mode...` and human-readable `list`/`history` formats have
regression tests, including an isolated D-Bus round trip. Production routing,
full output/action ordering, and other exact control behavior remain unfinished.

The extra `posthorn --experimental-private-desktop PATH` command is a
**development-only live preview**: it runs the private-name D-Bus daemon
alongside a separate Wayland renderer thread and updates a persistent stacked
popup surface whenever the notification store changes. It does not claim the
real desktop notification D-Bus name or replace Mako. Basic button/touch
handlers are implemented but compositor interaction, output routing,
activation and scaling now have headless protocol tests but are not yet
verified on a real Hyprland session. Do not autostart it.

## Automated compositor acceptance (development only)

CI runs `tests/headless_wayland.sh` inside a new `dbus-run-session` and an
isolated headless Sway compositor (wlroots). It launches the private
`org.posthorn.Development` desktop service, calls Freedesktop `Notify`,
observes layer-shell `configure` and `wl_surface.attach` protocol traffic,
then uses wlroots screencopy to capture the headless output and verify
that all three success/failure/warning category colors actually appear in newest-first order near the configured right anchor. This proves
basic compositor rendering and category color propagation, **not** pixel-perfect
Hyprland parity, fractional HiDPI, or production service ownership. The installed Mako daemon is never
involved.

## License

MIT. Mako's original authors are credited in [the source baseline](docs/SOURCE_BASELINE.md).
