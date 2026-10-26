# Posthorn 📯

**A Rust reimplementation of the Mako notification daemon.**

![Posthorn project color, warm amber-orange #E5923C](assets/posthorn-color.svg)

**Project color:** `#E5923C` · CSS `oklch(73% 0.14 63)` — [OKLab color decision](docs/COLOR.md).

Posthorn is being built as a **strict port of modern Mako first**. The MVP
inauguration requires matching Mako's real notification behavior, configuration,
D-Bus APIs, Wayland rendering, actions, history, and controls. Only **after**
that milestone will we add TOML, a graphical notification center with history
and Clear All, channels, SQLite persistence, and a general local message broker.

> **Not yet a production-ready Mako replacement.** Rust now includes configuration/criteria,
> notification lifecycle, timers, insertion-time grouping with persistent indices and active-list reordering, D-Bus
> adapters on an isolated development bus, a continuously refreshed
> Wayland layer-shell stack with release-aware SHM buffers and surface reuse,
> dynamic named output routing with native-resolution integer-scaled buffers, distinct outer/window and
> internal popup margins, directional padding and corner radii, HiDPI theme-aware icon selection and icon border-radius clipping, shrink-wrapped Cairo/Pango text and icons, configured
> post-grouping on-notify hooks, mouse/touch bindings with compositor-verified
> transparent input regions and atomic redraws, XDG activation tokens,
> ordered control list/history action dictionaries and CLI selection, Mako-style suppression of action signals under actions=0, and category-aware history,
> scoped group-by and reapplication on close/mode/reload, grouped count formatting, overrideable secondary-member visibility, accurate overflow placeholders, Mako's unknown urgency state, and partial posthornctl.
> **It does not claim the desktop notifications D-Bus name.** Inline
> Freedesktop image-data hints, Chromium numeric urgency variants, typed known-hint validation, progress fills and transactional config reload
> are implemented in the private development daemon. Exact Pango layout,
> fractional-scale behavior, full control parity, and live
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
cargo run -- --check-config
cargo run -- --check-config /path/to/mako/config
# In a temporary D-Bus session (never claims the real Mako bus name):
dbus-run-session -- cargo test --test dbus_wire --features dbus-integration
```

`--check-config` is a **development-only**, read-only parser command. Without PATH,
it follows Mako's legacy-before-XDG configuration lookup and uses Mako defaults
when no file is found. Success means the
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

The separate `posthorn --experimental-private-multi-desktop PATH` mode
also uses **only `org.posthorn.Development`**, but maps independent
Wayland layer-shell surfaces for distinct configured-output/layer/anchor
identities. It maintains separate compositor output observations and input
coordinates per surface, with independent notification stacks and
overflow placeholders. This remains opt-in developer QA, not an autostart
or production notification service. Keep installed Mako active.

## Automated compositor acceptance (development only)

CI runs `tests/headless_wayland.sh` inside a new `dbus-run-session`
and an isolated headless Sway compositor (wlroots). It launches the
private `org.posthorn.Development` service, sends Freedesktop `Notify`,
checks layer-shell configuration and attached SHM buffers, and uses
compositor screenshots to verify category colors, newest-first stacking,
right-anchor placement, timeout expiry, transactional reload, and
existing-notification restyling under `SetMode`.

The initially pointerless headless compositor receives a temporary
wlroots virtual pointer from an explicitly guarded Rust QA example.
Actual right-button events must dismiss only the targeted popup at
1× and 1.25×, with 2× testing both real pointer dismissal and the
independent Freedesktop `CloseNotification` repaint path. At 1×,
a left-button default action must receive a genuine asynchronous
compositor XDG activation token and emit the identical D-Bus
`ActivationToken` ahead of `ActionInvoked` and
`NotificationClosed`. Clicks outside popups and unbound middle
buttons must not dismiss notifications or request activation tokens.
The 1× suite also hotplugs a named headless monitor and verifies
recovery from output removal. See the [source-level acceptance record](docs/SOURCE_BASELINE.md)
and [CI run](https://github.com/sguzman/posthorn/actions/runs/40001253136).

This verifies **isolated Sway behavior**, not real Hyprland
pointer/touch or action focus, exact fractional-buffer negotiation,
physical audio, pixel-perfect Mako rendering or production service
ownership. The installed Mako daemon is never involved.

A separate [guarded private Hyprland acceptance runner](docs/HYPRLAND_QA.md)
is prepared for delegated automated workstation QA. It checks real-screen
popup appearance/removal using a disposable nested D-Bus session, without
changing Mako's bus ownership. **This runner has not yet been executed
on Hyprland**, so no real-session acceptance is claimed.

The [CI-backed multi-output acceptance run](https://github.com/sguzman/posthorn/actions/runs/40045890584)
also verifies simultaneous notification surfaces at 1×, 1.25× and 2×,
independent screenshots on two virtual monitors, and a compositor
right click that dismisses the second output's notification without
removing the first. The survivor remains visible after unplugging the
other monitor. These are automated isolated Sway results, not physical
Hyprland acceptance. A newly prepared, **explicitly armed** `--reversible-production-trial`
entrypoint can register the standard notification name only after an
authorization variable and exact session-bus pin are provided. Its
CI test runs entirely inside disposable software Sway and a new D-Bus
session. It is **not authorized for the workstation** and does not
change Mako, systemd, Hyprland autostart, or package-manager state.

The separate
[consent-gated reversible Mako cutover plan](docs/SAFE_CUTOVER.md)
defines the final MVP inauguration gate; no production switch is authorized.

## License

MIT. Mako's original authors are credited in [the source baseline](docs/SOURCE_BASELINE.md).
