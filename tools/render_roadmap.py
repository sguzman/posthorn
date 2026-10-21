#!/usr/bin/env python3
"""Render auditable project percentages from atomic task progress.

Usage: python tools/render_roadmap.py  (from repository root)
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA = json.loads((ROOT / "docs/progress.json").read_text())


def pct(tasks):
    return round(sum(t["progress"] for t in tasks) / len(tasks)) if tasks else 0


def bar(n):
    filled = round(n / 10)
    return "█" * filled + "░" * (10 - filled) + f" {n}%"


lines = ["# Posthorn roadmap", "", "Source of truth: [`progress.json`](progress.json).",
         "Progress is the average of documented atomic task percentages. A task at 100%",
         "requires acceptance evidence; partial work stays below 100%. Future ideas",
         "do not count toward Mako MVP. No runtime parity is claimed yet.", ""]
for m in DATA["milestones"]:
    tasks = [t for p in m["phases"] for t in p["tasks"]]
    lines.append(f"## {m['emoji']} {m['id']} — {m['name']} — `{bar(pct(tasks))}`")
    if "gate" in m:
        lines.append(f"**Acceptance gate:** {m['gate']}")
    lines.append("")
    for p in m["phases"]:
        lines.append(f"### {p['emoji']} {p['id']} — {p['name']} — `{bar(pct(p['tasks']))}`")
        lines.append("")
        for t in p["tasks"]:
            lines.append(f"- {t['emoji']} {t['id']} — {t['name']} — `{bar(t['progress'])}`")
        lines.append("")
    lines.append("**Feature inventory** (separate from task-based milestone percentage):")
    lines.append("")
    for f in m["features"]:
        lines.append(f"- {f['emoji']} {f['name']} — `{bar(f['progress'])}`")
    lines.append("")

(ROOT / "docs/ROADMAP.md").write_text("\n".join(lines))
