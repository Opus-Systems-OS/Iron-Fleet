---
name: blueweb-customer-site
description: Build, launch and hand over a website for a BlueWeb customer — scaffold an Astro site from the house template, create the private GitHub repo the customer gets access to, deploy it on Cloudflare Pages, and wire a backend only if the site actually needs one. Use when starting a new customer site, launching one, pointing a customer's domain at it, adding a contact form or other backend to one, or handing a finished site over.
---

# BlueWeb customer sites

BlueWeb builds websites for local businesses — barbershops, salons, auto
shops, gyms, detailers. This skill covers a customer's site from intake to
handover.

**The sales model shapes the engineering.** BlueWeb builds the whole site
first, shows it to the owner in person, and only then gets paid ($500 files
only, $700 built and launched, $700 + optional $99/month for ongoing changes
within three business days). So: the site must be genuinely finished before
anyone sees it, and the $99/month promise is only cheap to keep if a change
means editing one data file.

## Non-negotiables

- **No Netlify.** Netlify hosts BlueWeb's own marketing site (`BlueWebSite`)
  and nothing else. Customer sites are Cloudflare Pages. Do not add
  `netlify.toml`, Netlify Forms, or a Netlify deploy step to a customer repo.
- **Astro, static output, no framework.** A local business site is content.
  Reach for React/Svelte only if something genuinely needs client state.
- **The customer's repo is theirs.** Private, under the `BlueWeb-Org` org, and the
  customer is a collaborator on it.
- **The site must survive BlueWeb.** Everything the site *is* lives in the
  repo; the only things outside it are three accounts (GitHub, Cloudflare,
  Resend), each with a replacement documented in the repo's own `OFFLOAD.md`.
  A customer who cancels walks away with complete independent control. Adding
  a fourth external dependency — a hosted CMS, a booking service on a BlueWeb
  account, a BlueWeb-scoped npm package, an image CDN keyed to BlueWeb — turns
  "cancel any time" into a hostage situation. `references/offload.md` is the
  rule and the runbook.
- **Never push to `main`.** Cloudflare deploys `main` straight to the live
  domain. Branch, PR, check the preview, let the maintainer merge.
- **Secrets never enter the repo.** API keys are Cloudflare environment
  bindings. There is no `.env` in a customer repo.

## Workflow

### 0. Preflight — once, not per customer

Check before the first customer site, and re-check if any step below errors on
authentication. Full detail in `references/preflight.md` — and if `GH_TOKEN`
is already set in your environment, read its "In a Managed Agents sandbox"
section first: `git push` does not work there and the `github` MCP tools
replace it.

```sh
gh auth status                       # needs the `workflow` scope — see below
gh api user/orgs --jq '.[].login'    # needs to include `BlueWeb-Org`
npx wrangler whoami                  # Cloudflare account
```

Two things bite on a fresh machine:

- **The org is `BlueWeb-Org`, not `blueweb`.** Plain `blueweb` is an unrelated
  Slovak company's org, registered in 2011. Pushing a customer's repo there is
  not a typo you get to discover later. Never abbreviate it.
- **A token without the `workflow` scope cannot push `.github/workflows/`.**
  The push fails with `refusing to allow an OAuth App to create or update
  workflow`. Fix with `gh auth refresh -h github.com -s workflow`.

### 1. Intake

Get the business facts before writing anything. The checklist —
and what to do about the fields owners typically cannot answer on the spot —
is in `references/intake.md`. The template's `src/data/business.js` is
structured to match it field for field.

### 2. Scaffold

```sh
# Path is relative to this skill's directory, wherever it is mounted.
scripts/new-site.sh \
  --name "Kenn's Plumbing" --slug kenns-plumbing --domain kennsplumbing.com
```

Creates `~/Documents/BlueWeb/customers/<slug>` with a lockfile, a current CSP
hash and one git commit. Use the domain they *will* buy even if they have not
bought it yet — it is only used for canonical URLs.

### 3. Build the site

1. Fill in `src/data/business.js` from the intake notes. Nothing else in
   `src/` should hardcode a business fact.
2. Adjust components and add sections for what this business actually is. The
   template is a starting shape, not a ceiling.
3. `npm run dev` and read the site on a phone-width viewport first. Most
   visitors are standing on a sidewalk.

`references/build-conventions.md` has the constraints that will otherwise bite
you: the strict CSP, why there is no client JavaScript, and why
`npm run build` is not a reliable local check on macOS.

### 4. Create the repo

Private under `BlueWeb-Org`, pushed, with CI green. The customer is **not**
invited yet — BlueWeb shows the site in person and gets paid before handing
over access. Invite early only if the customer asks.

```sh
cd ~/Documents/BlueWeb/customers/<slug>
gh repo create BlueWeb-Org/<slug> --private --source=. --remote=origin --push
gh run list --limit 3          # CI should be green before anyone sees this
```

### 5. Deploy

Cloudflare Pages, connected to the repo, building `npm run build` into
`dist/`. The custom domain, the DNS, and the preview-deployment access policy
are all in `references/deploy-cloudflare.md`. Do this before the in-person
demo — showing a real URL on the owner's own phone closes better than a
laptop on their counter.

### 6. Backend, only if the site needs one

Default is none: a static site with a phone number. Add the contact form
(`functions/api/contact.js`, already in the template) when the owner wants
messages, and set the three Cloudflare bindings for **both** Production and
Preview. Bookings, menus that change daily, or anything with accounts are a
different conversation — `references/backend.md` covers what to reach for and
what to talk them out of.

### 7. Hand over

Invite the customer, walk them through the repo, and settle the domain
ownership. What changes between the $500, $700 and $99/month plans is in
`references/handoff.md`. The domain is bought by and stays with the customer
in every plan — that is a promise the site itself makes.

### 8. Offload, whenever they want it

Cancelling ends BlueWeb making changes; it does not take the site down and it
does not leave anything stranded. The repo ships `OFFLOAD.md`, written for the
customer and their next developer on the assumption BlueWeb cannot be asked
questions. `references/offload.md` has the runbook — transfer the repo, let
them stand up their own Cloudflare project and Resend key, verify *their*
deploy serves the domain, and only then delete anything of BlueWeb's. Do it
promptly; a clean exit is a referral.

## Maintaining a live site

A change request on the $99/month plan is: branch, edit `business.js`, PR,
check CI and the Cloudflare preview, report the preview URL, merge when told.
There is deliberately no extra command to remember — see
`references/build-conventions.md` on why the CSP carries no hash. If a request cannot be satisfied by editing
`business.js`, that is a signal the data file is missing a field — add it
there rather than hardcoding the value in a component.
