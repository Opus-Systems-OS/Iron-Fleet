# __BUSINESS_NAME__ — working notes

Astro static site for __DOMAIN__, built and maintained by BlueWeb. Hosted on
Cloudflare Pages. The only server-side code is `functions/api/contact.js`, a
Pages Function that turns the contact form into one email.

## The one file most changes touch

`src/data/business.js`. Name, phone, address, hours, services, prices, brand
colors. Nothing in `src/` outside that file should hardcode a business fact.
When the owner asks for "new hours" or "add a service", that is the edit.

That file also feeds the LocalBusiness structured data in
`src/data/schema.js`, which is what Google shows in search. So an edit to the
hours is an edit to the search result too — there is nothing else to run,
but it is worth knowing that a typo here is wrong in two visible places.

## Shipping changes

**Branch → PR → CI + Cloudflare preview → squash merge.** Never push straight
to `main` unless explicitly asked.

```sh
git checkout -b <topic>
# ...work, commit...
git push -u origin <topic>
gh pr create --fill
```

Cloudflare builds every branch and comments a preview URL on the PR. Report
the CI result and that preview URL, then let the maintainer merge with
`gh pr merge --squash --delete-branch`.

Two reasons this is not ceremony:

- **A push to `main` deploys to the customer's live domain, ungated.** This is
  a business's phone number and opening hours; a bad merge is a customer
  standing outside a closed door.
- **The security headers in `public/_headers` are not applied by
  `astro preview`.** A CSP regression is invisible locally and only shows up
  on the Cloudflare preview URL. Anything touching scripts, styles, fonts or
  the JSON-LD needs the preview checked, not just localhost.

## Commands

```sh
npm ci          # never `npm install` — the lockfile is authoritative
npm run dev     # localhost:4321
npm run build   # → dist/
```

`npm run build` works locally and takes well under a second on a site this
size. If it ever hangs instead — Astro's rolldown native binding has done this
on macOS in the BlueWeb marketing repo — do not sit waiting on it and do not
read the hang as a broken change. Kill it and lean on the PR's CI and
Cloudflare preview, which build on Linux, as the real signal.

Before concluding it hung, check you are not piping it: `npm run build | tail`
buffers every line until the process exits, so a build that is merely slow to
start is indistinguishable from a deadlock. Run it unpiped.

## Portability is a hard constraint

The customer can cancel at any time and take this site with them, and
`OFFLOAD.md` is the instructions for doing it without BlueWeb's help. That
only stays true if everything the site *is* lives in this repo.

Exactly three things are outside it: the GitHub repo host, the Cloudflare
Pages project, and the Resend API key behind the contact form. `OFFLOAD.md`
documents a replacement for each. **Do not add a fourth.** No hosted CMS, no
booking service on a BlueWeb account, no BlueWeb-scoped npm package, no image
CDN keyed to a BlueWeb login — content goes in `src/data/business.js`, images
go in `public/`, and secrets are per-site Cloudflare bindings so swapping in
someone else's key changes no code.

If a fourth dependency is genuinely unavoidable, it lands in `OFFLOAD.md` with
its replacement path in the same commit. CI fails if `OFFLOAD.md` is deleted.

## Constraints that bite

**The CSP is strict and served from `public/_headers`.** `script-src 'self'`
plus one SHA-256, for the JSON-LD. There is no `'unsafe-inline'`, so an inline
`<script>` will be refused by the browser with no visual clue on the page —
any real script goes in `public/js/` and is referenced with
`<script is:inline src="/js/…">`. `style-src-elem 'self'` likewise refuses
inline `<style>` elements, which is why `astro.config.mjs` sets
`inlineStylesheets: 'never'` and why styles live in `src/styles/global.css`.
`style=""` attributes are fine — that is how the brand colors get onto
`<html>`.

**The site has no JavaScript at all right now, including the form.** The
contact form is a plain `method="POST"` to a same-origin Pages Function, so it
works with scripts blocked. Keep it that way unless there is a real reason;
"submit without a page reload" is not one for a business that mostly wants
phone calls.

**Lightning CSS rewrites transforms at build time.** `translate3d(x, 0, 0)`
becomes `translateX(x)`, so the 3D syntax buys no compositor layer. Use
`will-change` if a layer is actually needed, and check `dist/_astro/*.css`
rather than the source before believing it shipped.

**The contact form's bindings live in the Cloudflare dashboard, not in this
repo.** `RESEND_API_KEY`, `LEAD_TO`, `LEAD_FROM` — and they must be set for
the Preview environment too, or the form 500s on every PR preview while
working fine in production.
