# Vendored jaack design system

Presentation package for book.jaack.me. No data, endpoints, or secrets.

- Version: Forma v2 (2026-11-09)
- Source: `iJaack/Jaack`, `assets/jaack-design-system/` at `e626247`.
- Vendored tree SHA-256 (SHA256SUMS bytes, screenshots excluded): `ea4e7b0a2db7653ffe56a82bce547e1b076cc7bca1dfa41264aaf95fbc95823a`

`SHA256SUMS` lists every canonical package file in sorted relative-path order,
using `<SHA-256>  <path>\n`. Hash its exact bytes with SHA-256 to reproduce the
tree digest above. Repository bookkeeping (`VENDOR.md`, `SHA256SUMS`, and
`SCREENSHOTS.sha256`) is excluded to avoid a self-referential checksum.
The original archive hash no longer identifies this Forma v2 tree.

Run `sh build.sh` to regenerate `jaack.css` and `jaack.js`; both match the
canonical bundles byte for byte. Served copies are also byte-identical:

- `public/assets/jaack/theme.js`
- `public/assets/jaack/jaack.css`
- `public/assets/jaack/jaack.js`

Site overrides live in `public/assets/jaack-local.css` and the theme switch
accessible-name fix in `public/assets/jaack-local.js`. They never patch vendor
sources. Canonical defaults retain the gold accent and veil tokens; booking
pages override the accent with plum and contain no veil markup.

`SCREENSHOTS.sha256` is retained from the original package. It describes the
original demo screenshots, which are not copied into this repo; it does not
claim refreshed Forma v2 screenshot evidence. Light is 07:00–18:59 local time
(`theme.js`, `data-light-from="7"`, `data-light-to="19"`).
