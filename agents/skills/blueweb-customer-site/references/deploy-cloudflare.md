# Deploying on Cloudflare Pages

One Cloudflare account holds every customer's Pages project. The project is
connected to the GitHub repo, so `git push` is the deploy — there is no manual
upload step and no deploy credential in the repo.

## Create the project

Dashboard → Workers & Pages → Create → Pages → Connect to Git → pick
`BlueWeb-Org/<slug>`.

| Setting | Value |
| --- | --- |
| Production branch | `main` |
| Build command | `npm run build` |
| Build output directory | `dist` |
| Root directory | (leave empty) |
| Node version | picked up from `.nvmrc` (22) |

Nothing else needs changing. `functions/` at the repo root is detected
automatically and deployed alongside the static build — that is the contact
form endpoint, and it needs no configuration of its own.

## Preview deployments

Cloudflare builds every branch and comments the preview URL on the PR. That
preview is the only place the real security headers apply, so it is the check
that matters for anything touching scripts, styles or fonts.

**Set the preview access policy.** Dashboard → the project → Settings →
General → *Preview deployment access* → require Cloudflare Access. Otherwise
every branch preview is a public, indexable copy of the customer's site at a
`*.pages.dev` URL, competing with their real domain in search. The canonical
`<link>` in `Base.astro` mitigates it; the access policy removes it.

## The custom domain

**The customer buys and owns the domain, in every plan.** That is a promise
the BlueWeb site makes in writing. Do not register it on a BlueWeb account
"to make it easier" — it is the one thing that makes the site genuinely
theirs, and it is the thing that makes leaving painless, which is why it is
worth saying out loud during the demo.

On the $99/month plan BlueWeb *manages* the domain. That means holding
delegated access, not ownership.

1. Customer registers the domain (Namecheap, Cloudflare Registrar, whoever).
2. Add the site as a zone in Cloudflare, and point the registrar's
   nameservers at the two Cloudflare gives you. This moves DNS, not
   ownership — the customer keeps the registrar account and can point the
   nameservers back at any time.
3. Pages project → Custom domains → add both `example.com` and
   `www.example.com`. Cloudflare creates the records and issues the
   certificate, usually within a few minutes.
4. Pick one as canonical and redirect the other with a bulk redirect or a
   redirect rule. Match `astro.config.mjs`'s `site` value — the apex, unless
   the customer has a reason to prefer `www`.
5. Confirm HTTPS works on both before showing anyone.

## Verifying a deploy

```sh
curl -sI https://example.com | grep -i -E 'content-security-policy|strict-transport'
```

If those headers are missing, `_headers` did not reach the root of the build
output — CI asserts `dist/_headers` exists for exactly this reason.

Then load the site in a browser and check the console is clean. A blocked
inline script or stylesheet shows up there and nowhere else.

## Logs

```sh
npx wrangler pages deployment tail --project-name <slug>
```

The only way to see why the contact form returned a 502. The Function logs the
Resend error there and deliberately does not put it on the page, because the
API response can quote account details.

## Rolling back

Dashboard → the project → Deployments → an earlier successful deployment →
Rollback. Instant, and it does not touch the repo. Do it first if a bad merge
reaches production, then fix forward in a PR.
