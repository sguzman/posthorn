# Guarded Hyprland acceptance

Sway CI verifies private Wayland protocols and screenshots but **does not**
prove real Hyprland behavior. The isolated, non-destructive
`tests/hyprland_private.sh` harness prepares one automated part of that
remaining acceptance gate.

It is designed to be run by the delegated automated QA workflow in a
real Hyprland session; no manual QA or production cutover is implied.
The explicit `POSTHORN_HYPRLAND_QA=1` guard must be set. Before doing
anything, the harness asks the **original session bus** for its current
`org.freedesktop.Notifications` owner. It refuses to proceed if the
name is not owned by the existing daemon.

Before building, the parent runs the guarded, read-only
`tests/read_only_owner_inventory.sh` probe to verify the existing Mako
executable/PID and classify its potential user-service supervisor
without printing personal configuration or command-line arguments.
An unverified supervisor does not authorize any production switch.
The parent requires the probe's rollback status to remain unverified.

The runner compiles the experimental private desktop and Rust PNG
pixel probe. It starts a **new nested D-Bus session**, retains the real
Wayland compositor socket, and exercises the **same multi-surface Wayland
rendering path used by the separately gated production trial**, while
running Posthorn on its private
`org.posthorn.Development` name using a disposable configuration.
Direct `--inside` invocation is rejected unless the parent has supplied
the original bus address and checked owner. The child also compares
the two bus daemon GUIDs to prove that its private session is **not**
the original bus, even if address strings are misleading. It asserts
that `org.freedesktop.Notifications` has **no owner** on the isolated
bus both before and after the private notification lifecycle. It also
resolves the unique owner of `org.posthorn.Development` via the
D-Bus daemon and checks that its process ID matches the exact
Posthorn process started by the harness. Unexpected public-name
ownership or a foreign private owner fails QA without touching the
original session.

Before launching Posthorn, the runner captures an ephemeral baseline
screenshot. It sends a Freedesktop `Notify`, verifies the resulting
ID and active private control entry, then requires the distinct popup
color to **increase relative to the original screenshot**. While the original notification remains active, the harness edits only
its disposable private config and calls `fr.emersion.Mako.Reload`. It
checks that the existing popup acquires the new background color and
loses the old one. After `CloseNotification`, it verifies both that the
control entry disappears and that the final matched pixel count returns
close to its original baseline. These comparisons reject false positives
from an existing wallpaper or application that already uses the QA
colors. Compositor queries and screenshot captures have explicit timeouts;
the runner terminates its child process and deletes temporary captures
on exit.

An opt-in `POSTHORN_KEEP_QA=1` preserves artifacts for private
debugging only; **never commit screenshots or user-session logs to
this public repository**.

All application/configuration side effects are isolated. The script
must never claim the original bus's notification name, modify the
installed Mako or Hyprland configuration, alter autostart, or issue a
production cutover. The screenshot probe uses Rust/GDK-Pixbuf rather than
Python.

## Optional native-compositor CI experiment

The separate `Experimental Headless Hyprland` workflow uses a disposable
Arch Linux container to compile Posthorn against a current Hyprland package.
The test harness in `tests/headless_hyprland.sh` is explicitly CI-guarded,
runs under a disposable unprivileged account, uses a private
`dbus-run-session`, and does not connect to the workstation.
It attempts to initialize Hyprland over a nested software-rendered
Sway Wayland backend (rather than requiring a GPU passed through the
container) and then checks a private D-Bus notification's appearance
and dismissal via real compositor screenshots.

This experiment is **not a required main CI gate** and is now triggered
**only by explicit workflow dispatch**. Early isolated Arch/GitHub container
runs compiled Posthorn and installed Hyprland, but Hyprland could not
initialize its Aquamarine backend (`CBackend::create() failed`).
Neither a shorter IPC path nor a software-rendered nested Sway host
resolved that container graphics limitation. This is **not** a failed
Posthorn compositor test, because the Hyprland Wayland socket never
became available.

The experimental workflow no longer suppresses failures with
`continue-on-error`. When explicitly dispatched on a suitable
graphics-capable runner, only a completed **successful native compositor
lifecycle step** may become acceptance evidence. A green ordinary Rust/Sway
workflow does not imply that this step passed.

Hyprland 0.55+ uses
[Lua configuration](https://wiki.hypr.land/Configuring/Start/);
its [virtual-headless-output documentation](https://wiki.hypr.land/Configuring/Extra/Virtual-GPU/)
describes the general virtual display capability. CI host setup, including
the available Wayland backend and security sandbox, is not guaranteed
by those documents. Failed compositor bootstrap attempts are
environment findings, not Posthorn feature failures.

## Remaining acceptance

Successful execution would establish only real Hyprland private
popup appearance, disappearance and D-Bus lifecycle while preserving
the original notification daemon. It would **not** establish physical
pointer/touch input, XDG activation-induced focus, physical sound,
monitor hotplug and scaling parity, exact Mako rendering, or permission
to replace installed Mako.

The harness is currently **not executed on a real Hyprland session**.
Adding it to the repository or compiling the image probe does not
establish T503 completion. The existing private Sway CI remains the
only compositor evidence until an actual guarded Hyprland run occurs.
