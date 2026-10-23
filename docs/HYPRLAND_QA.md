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

The runner compiles only the experimental private desktop and Rust PNG
pixel probe. It starts a **new nested D-Bus session**, retains the real
Wayland compositor socket, and runs Posthorn on its private
`org.posthorn.Development` name using a disposable configuration. It
then sends a Freedesktop `Notify`, captures a temporary real-monitor
screenshot, checks for the unique notification background color, closes
the notification through the private bus, and checks that its color
disappears. The runner terminates its child process and deletes temporary
captures on exit. An opt-in `POSTHORN_KEEP_QA=1` preserves artifacts for
private debugging only; **never commit screenshots or user-session logs to
this public repository**.

All application/configuration side effects are isolated. The script
must never claim the original bus's notification name, modify the
installed Mako or Hyprland configuration, alter autostart, or issue a
production cutover. The screenshot probe uses Rust/GDK-Pixbuf rather than
Python.

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
