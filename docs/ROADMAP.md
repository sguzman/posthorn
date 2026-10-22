# Posthorn roadmap

Source of truth: [`progress.json`](progress.json).
Progress is the average of documented atomic task percentages. A task at 100%
requires acceptance evidence; partial work stays below 100%. Future ideas
do not count toward Mako MVP. No runtime parity is claimed yet.

## 📯 M0 — Mako parity and MVP inauguration — `████████░░ 83%`
**Acceptance gate:** Full upstream parity + Hyprland end-to-end QA

### 🧭 P0 — Source baseline and safeguards — `██████████ 99%`

- 📌 T001 — Pin Mako v1.11.0 and document newer delta — `██████████ 100%`
- 🎨 T002 — Assign OKLab/OKLCH Posthorn color and embed README swatch — `██████████ 100%`
- 🔐 T003 — Keep live Mako untouched, isolate public test fixtures — `██████████ 100%`
- 🧪 T004 — Validate Rust scaffold with CI and formatting — `██████████ 97%`

### 🧩 P1 — Config, criteria and types port — `████████░░ 83%`

- 📄 T101 — Parse Mako configuration sections and ordered overrides — `██████████ 100%`
- 🔎 T102 — Match categories, modes, exact fields, quoted and regex selectors — `█████████░ 92%`
- 🧰 T103 — Complete option validation, built-in grouping and escaping semantics — `███████░░░ 71%`
- 📂 T104 — Include files with Mako-compatible path behavior — `████████░░ 79%`
- 📐 T105 — Type/layout/style parity for all Mako config settings — `███████░░░ 74%`

### 📨 P2 — Notification lifecycle and formatting — `█████████░ 93%`

- 🆔 T201 — Allocate nonzero IDs, replacement by ID and app-scoped tag — `██████████ 100%`
- 🗃️ T202 — Bounded in-memory history, close and restore — `██████████ 100%`
- ⏳ T203 — Timeout selection and timer/event scheduling — `█████████░ 94%`
- 🔗 T204 — Grouping, sorting, hidden placeholder and visibility — `██████████ 96%`
- 🔤 T205 — Pango markup/escaping, format specifiers and actions — `████████░░ 77%`

### 🚌 P3 — D-Bus protocol and control surface — `████████░░ 84%`

- 📬 T301 — Implement org.freedesktop.Notifications methods and signals — `██████████ 95%`
- 🧾 T302 — Implement full Mako D-Bus control interface, properties and JSON — `█████████░ 87%`
- 🔄 T303 — Correct startup, bus ownership and config reload behavior — `███████░░░ 69%`

### 🪟 P4 — Wayland protocol and rendering — `█████████░ 90%`

- 🧱 T401 — Layer-shell surfaces, anchors, output routing and scaling — `██████████ 95%`
- 🖌️ T402 — Text, icons, colors, borders, progress and layout — `██████████ 99%`
- 🖱️ T403 — Pointer, touch, button bindings and XDG activation — `████████░░ 75%`

### 🎛️ P5 — CLI and exact workstation parity — `█████░░░░░ 51%`

- ⌨️ T501 — Port makoctl operations with compatibility tests — `█████████░ 86%`
- 🔔 T502 — Reproduce category colors, per-category sounds and quiet mode — `███████░░░ 68%`
- 🧪 T503 — End-to-end on Hyprland, notify-send and Edge activation — `█████░░░░░ 49%`
- 🚀 T504 — MVP inauguration and opt-in reversible Mako switch — `░░░░░░░░░░ 0%`

**Feature inventory** (separate from task-based milestone percentage):

- 🔔 Desktop notification parity — `█████████░ 93%`
- 🎨 Mako category coloring and sound parity — `███████░░░ 72%`
- 📜 Mako history/control API parity — `█████████░ 89%`

## 📑 M1 — Post-MVP TOML configuration — `░░░░░░░░░░ 0%`

### 🧱 P6 — TOML compatibility bridge — `░░░░░░░░░░ 0%`

- 🔁 T601 — Round-trip converter for existing Mako config — `░░░░░░░░░░ 0%`
- 📝 T602 — Native TOML schema, validation and migration — `░░░░░░░░░░ 0%`

**Feature inventory** (separate from task-based milestone percentage):

- 📑 TOML configuration — `░░░░░░░░░░ 0%`

## 🪟 M2 — Post-MVP GUI notification center — `░░░░░░░░░░ 0%`

### 🖼️ P7 — Native history panel — `░░░░░░░░░░ 0%`

- 🗂️ T701 — Browse and activate previous notifications — `░░░░░░░░░░ 0%`
- 🧹 T702 — Clear all, dismiss, grouping and keyboard navigation — `░░░░░░░░░░ 0%`

**Feature inventory** (separate from task-based milestone percentage):

- 🪟 Graphical notification history and clear all — `░░░░░░░░░░ 0%`

## 📡 M3 — Post-MVP channels and general IPC — `░░░░░░░░░░ 0%`

### 📨 P8 — Local broker protocol — `░░░░░░░░░░ 0%`

- 🛤️ T801 — Namespace/channel ontology, publish and subscribe — `░░░░░░░░░░ 0%`
- 🔒 T802 — Unix socket protocol and permission model — `░░░░░░░░░░ 0%`

**Feature inventory** (separate from task-based milestone percentage):

- 📡 Application-to-application channels — `░░░░░░░░░░ 0%`

## 🗄️ M4 — Post-MVP SQLite persistence — `░░░░░░░░░░ 0%`

### 💽 P9 — Durable storage — `░░░░░░░░░░ 0%`

- 🗃️ T901 — SQLite schema, migration and retention policy — `░░░░░░░░░░ 0%`
- ⏪ T902 — Durable subscription cursor and message replay — `░░░░░░░░░░ 0%`

**Feature inventory** (separate from task-based milestone percentage):

- 🗄️ SQLite message storage — `░░░░░░░░░░ 0%`

## 🔀 M5 — Post-MVP message broker capabilities — `░░░░░░░░░░ 0%`

### ⚙️ P10 — Reliable delivery and operations — `░░░░░░░░░░ 0%`

- 📬 T1001 — Consumer groups, acknowledgments and retries — `░░░░░░░░░░ 0%`
- 🔍 T1002 — Observability, searchable streams and policy controls — `░░░░░░░░░░ 0%`

**Feature inventory** (separate from task-based milestone percentage):

- 📥 Work queues and fan-out delivery — `░░░░░░░░░░ 0%`
- 🧭 Monitoring and administrative controls — `░░░░░░░░░░ 0%`
