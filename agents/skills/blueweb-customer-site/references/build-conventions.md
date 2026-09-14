# Build conventions

House rules for a customer site. Most of these exist because something broke
silently once.

## Everything comes from `business.js`

No component hardcodes a business fact. This is what makes the $99/month
promise sustainable: "we're opening an hour earlier" is a one-line edit, not a
hunt through markup. If a change request cannot be satisfied by editing
`src/data/business.js`, add the field there rather than inlining the value.

`src/data/schema.js` reads the same object to build the LocalBusiness
structured data, so the hours in the table and the hours Google shows can
never disagree.

## Nothing may live outside the repo

Content in `business.js`, images committed to `public/`, secrets as per-site
Cloudflare bindings. The customer's exit path depends on it and CI asserts the
document describing that path still exists. `offload.md` has the full rule.

## The CSP is strict, and it fails silently

Served from `public/_headers`, which Astro copies verbatim into `dist/`.

- **`script-src 'self'`** — no `'unsafe-inline'`, no hash list, nothing to
  maintain. An inline `<script>` is refused by the browser with nothing
  visible on the page to tell you.
- **Real scripts go in `public/js/`** and are referenced with
  `<script is:inline src="/js/…">`.
- **`style-src-elem 'self'`** refuses inline `<style>` elements, which is why
  `astro.config.mjs` sets `inlineStylesheets: 'never'` — under the default,
  Astro inlines a small enough stylesheet and the browser then drops it.
  Styles live in `src/styles/global.css`.
- **`style=""` attributes are fine.** That is how the per-customer brand
  colors reach the stylesheet, as CSS custom properties on `<html>`.

**None of these headers apply to `astro dev` or `astro preview`.** A CSP
regression is invisible locally and only appears on the Cloudflare preview
URL. Anything touching scripts, styles, fonts or structured data needs the
preview checked.

## The JSON-LD needs no hash — do not add one

`Base.astro` renders one inline `<script type="application/ld+json">`, the
LocalBusiness structured data. It is tempting to think a strict `script-src`
needs a SHA-256 for it. It does not: browsers do not apply `script-src` to
`application/ld+json`, because it is data and is never executed.

This was measured, not assumed. On 2026-09-10, against Chrome with CSP
violation reporting enabled: a page whose only inline script was JSON-LD
reported **zero** violations under a deliberately wrong hash, while an
executable inline script on the same page under the same policy was blocked
and reported. Structured data also reaches Google from the served HTML, and
Googlebot does not enforce CSP.

An earlier version of this template did pin a hash, with a sync script and two
CI steps. It was removed because the cost landed in exactly the wrong place:
the hash changes whenever the business's hours or phone number change, which
is the single most common edit on the maintenance plan, so the most routine
change in the business carried a mandatory extra command and a red CI run when
someone forgot. If you find yourself reaching for a hash again, re-read this
section first.

## No client JavaScript

The template ships zero JS, including the contact form, which is a plain
`method="POST"` to a same-origin Pages Function. Keep it that way unless there
is a real reason. A local business site is read on a phone with one bar of
signal; every script is a thing that can fail to arrive.

That also means no hamburger menu — on narrow screens the section links
collapse and the phone number stays, because calling is the conversion.

## Accessibility floor

Not optional, and cheap at this size: real `<table>` for hours, labelled form
fields, a skip link, visible focus rings, 3rem minimum tap targets, 4.5:1 on
body text. A local business site gets read by people with old eyes on bright
sidewalks.

## Lightning CSS rewrites transforms

`translate3d(x, 0, 0)` collapses to `translateX(x)` at build time, so the 3D
syntax buys no compositor layer. Use `will-change` if a layer is genuinely
needed, and check `dist/_astro/*.css` rather than the source before believing
a compositing fix shipped.

## Building

`npm run build` is fast and works locally. If it ever hangs instead — Astro's
rolldown native binding has done this on macOS in the BlueWeb marketing repo —
kill it rather than waiting, and lean on CI and the Cloudflare preview, which
build on Linux.

First make sure it actually hung: `npm run build | tail` buffers all output
until the process exits, so a slow build and a deadlock look identical — both
show nothing. Run the build unpiped before believing it.

Never `npm install` after scaffolding. The lockfile is authoritative; `npm ci`
is the install.
