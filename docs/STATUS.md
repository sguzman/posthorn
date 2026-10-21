# Status reporting contract

For every development report:

1. Identify repository, branch, and verified commit(s).
2. Show the overall **M0 MVP** percentage bar, plus the percentage bar for
   **every touched milestone, phase, task, and feature**.
3. Distinguish implementation from compilation, isolated D-Bus wire-test CI, local Wayland QA and
   actual working behavior. Never equate a committed file with runtime parity.
4. Report the exact changed files, accepted tests, untested behavior, and
   next blockers without hiding failed checks.
5. Keep future features separate from Mako MVP percentages. Do not begin
   post-MVP work before the inauguration gate.
6. Work directly on `main` for Posthorn unless given a Posthorn-specific
   branch exception. Do not change another repository or live system as a
   side effect of Posthorn work.

Atomic task percentages are authoritative in `docs/progress.json`. Render
with `python tools/render_roadmap.py` whenever they change.
