# Reversible Mako-to-Posthorn acceptance gate

**Status:** documentation and readiness only. No production cutover is
authorized, scheduled, or executed. This is the **M0 inauguration gate**,
not a post-MVP project. `T504` remains **0%** until an actual explicitly
authorized and evidence-backed acceptance run.

## Invariants

- Never replace or queue for `org.freedesktop.Notifications` while Mako
  still owns that name. The private Posthorn QA name is
  `org.posthorn.Development`.
- Never alter the installed `mako` binary, its config, Hyprland autostart,
  package-manager state, or long-lived user services as a prerequisite
  to basic private QA.
- Never assume how the existing daemon starts. It might be supervised
  by systemd, launched by Hyprland, spawned from a shell, or activated by
  another mechanism.
- Preserve the user's full existing notification configuration, hooks,
  audio routing, default service, and session environment. Do not migrate
  a private copy of credentials or personal messages to public GitHub.
- The acceptance run and any rollback must be performed by automated QA.
  Do not turn the user into a manual QA operator.
- Compositor-real acceptance is mandatory. Synthetic Rust tests and
  isolated headless Sway do not prove the user's Hyprland, displays,
  touch devices, audio, or application focus behavior.

## Phase A: read-only workstation inventory

**Allowed without cutover authorization**, when automated QA has direct
access to the workstation and the user has authorized that private QA
access:

1. Record the exact existing owner of
   `org.freedesktop.Notifications` through the current **user bus**.
   Record Mako's actual running process, start mechanism, executable,
   relevant systemd unit (if any), and read-only Hyprland launch
   configuration. Fail if the service owner cannot be identified.
2. Record the actual monitor names, effective scale, layout, anchor,
   compositor protocol support, relevant focus policy, and the
   notification config path. Keep all raw workstation output private.
3. Compile Posthorn in its own checkout. Run the guarded
   [private Hyprland QA](HYPRLAND_QA.md) against a **nested D-Bus
   session**; verify Mako's original bus owner remains identical
   before and after that test.
4. Run the rest of the strict MVP real-device tests without claiming
   the standard notification name: compositor click and focus
   activation, physical touch, scaling/hotplug, notification timers,
   mode/reload transactions, and configured audio. Collect minimal
   synthetic acceptance evidence without personal notifications or
   unapproved screen recordings.
5. Require an evidence inventory of every source-pinned Mako v1.11
   behavior and failed/untested gates. No percentage may jump to 100%
   based on documentation alone.

Any failed invariant leaves the existing setup unchanged. Do not
silently substitute a different notification daemon or omit failed
physical-device checks.

## Prepared CLI capability (not authorization)

The development binary has a **non-default** `--reversible-production-trial`
entrypoint that reuses the multi-output Wayland renderer and standard
`org.freedesktop.Notifications` D-Bus interface. It will not start
unless both `POSTHORN_PRODUCTION_TRIAL_AUTHORIZED=1` and
`POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS` equal to the active
`DBUS_SESSION_BUS_ADDRESS` are supplied by an authorized automated
controller. It explicitly refuses private Hyprland QA mode.

A third mandatory guard, `POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS`,
must contain a finite integer from **1 to 600 seconds**. Missing,
malformed, zero or excessive durations are rejected before registration.
The daemon uses a monotonic deadline that is not extended by incoming
notifications or Reload, and releases its D-Bus name when the deadline
expires even if the controller has stopped responding. This is a
second line of defense, **not** a substitute for restoring Mako:
a real workstation switch still requires a separately supervised,
machine-local rollback mechanism.

The D-Bus name request uses `DoNotQueue`: an existing Mako owner cannot
be displaced and Posthorn cannot wait behind it to acquire the name
later. An isolated headless Sway and `dbus-run-session` test checks
both negative guards and reversible owner lifecycle. Passing that
test proves **only** that the binary can perform an isolated,
explicitly armed trial. It does not supply user authorization,
inventory the real Mako supervisor, provide a machine-local
rollback controller, or satisfy physical Hyprland acceptance.
Do not set these opt-in variables or run a production-name trial
against the user's real session before the separate Phase B gate.

## Phase B: explicit production-switch authorization

Before any Mako process is stopped or standard bus ownership changes,
obtain a separate, unmistakable affirmative instruction to run the
**reversible production switch**. Approval of private QA alone is
insufficient. Do not infer consent from general requests to keep
developing Posthorn.

The authorized automated controller must first create and verify a
machine-local rollback recipe reflecting the actual owner and supervisor
found in Phase A. It must preserve the exact previous daemon start
method and capture Mako's current executable and config locations.
Prepare a fallback that survives controller failure, where a suitable
supervision mechanism exists; otherwise **do not cut over**.

## Phase C: bounded trial and automatic rollback

Only after Phase B approval:

1. Gracefully release Mako's standard bus name through its actual
   recorded supervisor. Do not use blind `killall` or broad `pkill`
   against unidentified processes.
2. Launch the freshly built Posthorn standard-notification service in
   the same intended user session and verify it owns
   `org.freedesktop.Notifications` without queueing or displacing any
   other owner. Confirm the owner belongs to that exact launched
   instance.
3. Use disposable, clearly labeled test notifications to verify
   `Notify`, `CloseNotification`, action ordering, expiry, history,
   configured fonts/colors, real compositor events and audio.
   Verify normal applications can connect through the standard name.
4. If ownership, any acceptance test, or the controller fails, terminate
   **only the recorded trial instance**; restore Mako using its captured
   start mechanism; confirm its original service behavior and owner
   (which may have a *different unique bus name* after a restart).
   Report the failed gate and leave the user on Mako.
5. Even on passing tests, keep a guaranteed rollback path and explicit
   instruction-based finalization gate. Do not alter persistent
   autostart or uninstall Mako as a side effect of the trial.

The exact commands and rollback mechanism depend on the read-only
inventory. This document intentionally does **not** offer blind
workstation-mutating shell commands.

## Current nonclaims

As of 2026-10-23, the complete 1×/1.25×/2× headless Sway suite
and private D-Bus CI run successfully, and a guarded private
real-Hyprland test harness is prepared. The native Hyprland
container experiment could not initialize its graphics backend.
There is **no** verified physical workstation execution, physical
touch/audio acceptance, or authorized standard-name Mako cutover.
These gates must stay open in `docs/progress.json`.
