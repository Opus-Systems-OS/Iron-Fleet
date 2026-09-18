# Preflight — accounts and access

Once per machine, plus whenever a step in the workflow fails on auth. None of
this is per-customer.

## In a Managed Agents sandbox (Iron Fleet)

If `GH_TOKEN` and `CLOUDFLARE_API_TOKEN` are already set when you start, you
are running as the `blueweb-client` fleet agent in a cloud sandbox, not on a
laptop. Everything below still applies, with four differences:

- **No logins.** `gh auth status` and `npx wrangler whoami` already work; there
  is no device flow and no browser. If either fails, stop and report it — the
  fix is on the control plane (a rotated token), not something you can do here.
- **The token values are placeholders.** The sandbox only ever sees an opaque
  stand-in; the real secret is substituted at the network edge, and only in
  request *headers* to `github.com` / `api.github.com` / `api.cloudflare.com`.
  Never echo, paste, or write them into a file — they are useless anywhere else.
- **`git push` cannot work here — push with `scripts/push-tree.mjs`.**
  GitHub's git-over-HTTPS endpoint accepts only HTTP Basic auth, which
  base64-encodes the token, so the placeholder is never substituted and every
  `git push`/`git fetch`/`gh repo create --push` fails with
  `remote: invalid credentials` (verified 2026-09-14; Bearer/`token` headers
  are rejected by GitHub even with a real token). The REST API is fine —
  `gh api` sends the token in a header — so the skill ships a script that
  pushes a commit through the git-data API:

  1. `gh repo create BlueWeb-Org/<slug> --private --description "…"` (the
     REST half works). No `--source`/`--push`.
  2. Scaffold and commit locally as usual — the local repo is your working
     copy; what lands on GitHub is what CI and the Cloudflare preview build.
  3. `node <skill dir>/scripts/push-tree.mjs BlueWeb-Org/<slug> main`.
     It uploads every blob of `HEAD` from disk (base64, mode preserved),
     creates the tree, commit and ref, and **fails unless GitHub's tree SHA
     equals `git rev-parse HEAD^{tree}`** — which proves every byte on
     GitHub matches the local commit. An empty repo is seeded with `.nvmrc`
     first (the git-data API refuses trees on a repo with no commits). A
     branch that does not exist is created from the default branch (or
     `--from <branch>`); one that does is fast-forwarded. Commit message
     defaults to `HEAD`'s; `-m` overrides. Verified 2026-09-18 on both an
     empty repo and a new branch: 26 files, CI triggered and green.
  4. Later changes: commit, `push-tree.mjs … <branch>`, then
     `gh pr create`; `gh run list` / `gh pr checks` for CI. GitHub is
     canonical after a push — the remote commit is a different object from
     the local one (same tree), so do not expect `git log origin/…` to
     show it.

  **Do not push file contents through the `github` MCP `push_files` tool.**
  It works, but every byte goes through the model: the 2026-09-18 demo site
  spent $9.93 of a $10 cap, most of it transcribing a 143 KB
  `package-lock.json`. `push_files` is acceptable only for a one-line edit
  to a small file when the script is somehow unavailable. Never paste a
  token into any call.
- **Mounted skill files have no exec bits.** The upload is multipart, which
  carries no modes, so `scripts/new-site.sh` arrives `644` — run it as
  `bash …/new-site.sh`, never directly (`Permission denied`, 2026-09-18).
- **Work under `/workspace`.** Repositories you were given are already cloned
  there; scaffold new sites with `--dir /workspace/customers`, not
  `~/Documents`. Cloudflare Pages is connected through the dashboard by a
  human, exactly as in `deploy-cloudflare.md`; `wrangler` here is for
  `pages deployment tail` only.

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
