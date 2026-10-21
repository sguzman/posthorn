# Posthorn 📯

**A Rust reimplementation of the Mako notification daemon.**

![Posthorn project color, warm amber-orange #E5923C](assets/posthorn-color.svg)

**Project color:** `#E5923C` · CSS `oklch(73% 0.14 63)` — [OKLab color decision](docs/COLOR.md).

Posthorn is being built as a **strict port of modern Mako first**. The MVP
inauguration requires matching Mako's real notification behavior, configuration,
D-Bus APIs, Wayland rendering, actions, history, and controls. Only **after**
that milestone will we add TOML, a graphical notification center with history
and Clear All, channels, SQLite persistence, and a general local message broker.

> **Not yet a working daemon.** Current Rust implements initial config and
> notification lifecycle logic; there is no D-Bus service or Wayland window.
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
```

`--check-config` is a **development-only** parser command. Success means the
parser recognized the file; it does *not* mean the current Posthorn renderer
would reproduce every setting. Current features remain incomplete until
integration tests demonstrate behavior.

## License

MIT. Mako's original authors are credited in [the source baseline](docs/SOURCE_BASELINE.md).
