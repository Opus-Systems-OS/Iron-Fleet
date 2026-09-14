# Preflight — accounts and access

Once per machine, plus whenever a step in the workflow fails on auth. None of
this is per-customer.

## GitHub

```sh
gh auth status
gh api user/orgs --jq '.[].login'
```

**The org is `BlueWeb-Org`.** Customer repos live there, not on a personal
account.

```sh
gh api orgs/BlueWeb-Org --jq .login          # should print BlueWeb-Org
gh api user/memberships/orgs/BlueWeb-Org --jq .role   # needs admin:org to read
```

**`blueweb` is not us.** The bare name belongs to an unrelated Slovak company
(blueweb.sk), registered in 2011 and holding public repos. A customer site
pushed there is a private business's site published to a stranger's
organisation. Always write `BlueWeb-Org` in full — in `gh repo create`, in
collaborator invites, in transfer commands, and in the clone URL that ships in
every customer's `OFFLOAD.md`.

Verified 2026-09-10: `BlueWeb-Org` exists, the account is an active admin of
it, and members can create repositories.

**The `workflow` scope.** A token without it cannot push
`.github/workflows/`, and the failure arrives at the end of the first push:

```
! [remote rejected] main -> main (refusing to allow an OAuth App to create or
  update workflow `.github/workflows/ci.yml` without `workflow` scope)
```

```sh
gh auth refresh -h github.com -s workflow
```

Granted on 2026-09-10; the token now carries `gist, read:org, repo, workflow`.

**If you ever need to run that again, do it in a separate terminal window.**
`gh auth refresh` uses GitHub's device flow — it prints a one-time code and
then polls while you authorize in a browser. That cannot complete inside
Claude Code's 120-second Bash timeout, even with the `!` prefix: the command
gets backgrounded and the poll connection is reset. The non-interactive
alternative is a classic token with `repo`, `workflow`, `read:org`, `gist`
piped into `gh auth login --with-token`.

Only `repo` scope is needed to invite an outside collaborator to a single
repo, so the customer invite works without `admin:org`.

## Cloudflare

One Cloudflare account holds every customer's Pages project. `wrangler` is not
installed globally on purpose — use `npx wrangler`, which pins nothing and
leaves no stale binary.

```sh
npx wrangler whoami        # opens a browser login the first time
```

The deploy itself is configured through the Cloudflare dashboard (Pages
project connected to the GitHub repo), not through wrangler. Wrangler is here
for reading logs — `npx wrangler pages deployment tail` is the only way to see
why a contact form returned 502.

## Resend

Only needed for sites with a contact form. One Resend account, one verified
sending domain per customer. Free tier is 3,000 emails/month and 100/day,
which no local business contact form will approach.

Domain verification is DNS work on the customer's domain and takes a few
minutes to propagate — do it when the domain is set up, not on demo day. See
`backend.md`.

## What is deliberately absent

- **No Netlify.** Netlify hosts BlueWeb's own marketing site and nothing else.
- **No shared password manager entry per customer.** The only credential the
  site depends on is the Resend API key, which lives in Cloudflare's encrypted
  environment bindings and is never in the repo.
- **No analytics account per customer.** When an owner asks how many people
  visited, the zone-level traffic analytics in the Cloudflare dashboard answer
  it with no client-side script at all, because their domain is proxied
  through Cloudflare anyway. Cloudflare *Web* Analytics is a different thing —
  it injects a beacon from `static.cloudflareinsights.com`, which the site's
  CSP blocks until `script-src` and `connect-src` are widened for it. Do not
  widen the CSP for a number the dashboard already shows.
