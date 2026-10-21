# Project color — perceptually selected

Posthorn's project accent is **warm amber-orange**:

- Hex sRGB: `#E5923C`
- CSS: `oklch(73% 0.14 63)`
- Approximate OKLab: `L=0.73, a=0.0636, b=0.1247`
- Machine-readable home: `assets/posthorn-color.svg` (swatch is embedded in README)

The choice was made in OKLCH, using perceptual lightness/chroma/hue rather
than HSL/RGB steps. The result is within the sRGB gamut and contrasts with
Posthorn's dark README mark. This is a **locally selected project identity**;
it has *not* been checked for maximal perceptual distance from a global
registry of other project colors. If Posthorn joins a shared color registry,
validate pairwise OKLab distances and visually confusable neighbors there.

Color expresses project identity, **not** runtime severity. Existing Mako
notification colors and category routing remain independent of this accent.
