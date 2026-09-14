# Offload — giving a customer complete independent control

A customer who cancels must end up able to run their site with no BlueWeb
account, no BlueWeb credential and no BlueWeb goodwill. This is a design
constraint on every site, not just a procedure for the day they leave.

`OFFLOAD.md` ships in every customer repo and is written for the customer and
their next developer, on the assumption BlueWeb cannot be asked questions.
Keep it accurate as the site grows — an offload document that is wrong is
worse than none.

## The portability invariant

**Everything the site *is* lives in the repo. Everything outside it is an
account with a documented replacement.**

There are exactly three external dependencies, and the number should stay
three:

| Outside the repo | Why it is safe | Replacement |
| --- | --- | --- |
| GitHub repo, in the `BlueWeb-Org` org | Ordinary git; full history clonable by any collaborator | Transfer, or push a clone anywhere |
| Cloudflare Pages project | Holds no content — builds from the repo | Recreate on their account, ~5 min |
| Resend API key | Only the contact form depends on it | Their own free Resend account |

Before adding anything to a customer site, ask what it does to that table. A
booking system on a BlueWeb account, a CMS with BlueWeb-owned content, a
design system published under a BlueWeb npm scope, an image CDN keyed to a
BlueWeb account — each one turns "cancel any time" into a hostage situation.
If a fourth row is genuinely necessary, it must land in `OFFLOAD.md` with its
replacement path in the same commit.

Corollaries that are easy to get wrong:

- **Content lives in the repo**, not in a hosted CMS. `business.js` is the
  content store precisely because it travels.
- **Images live in `public/`**, committed, not on a BlueWeb-keyed CDN.
- **No BlueWeb npm package.** The template is copied into each site, not
  installed. Sites diverge over time — that is the intended cost.
- **Secrets are per-site Cloudflare bindings**, so a customer swapping in their
  own key changes no code.

## The offload runbook

Do this promptly and without friction. A local business owner tells other
local business owners, and a clean exit is a referral.

### 1. Transfer the repository

```sh
gh api -X POST repos/BlueWeb-Org/<slug>/transfer -f new_owner=<their-account>
```

They must accept. If they have no GitHub account and do not want one, give
them a bundle instead — it is a complete repository in one file:

```sh
git -C ~/Documents/BlueWeb/customers/<slug> bundle create <slug>.bundle --all
```

`git clone <slug>.bundle` restores everything, history included.

### 2. Hand over hosting

Either they recreate the Pages project on their own Cloudflare account
(`OFFLOAD.md` walks them through it), or — if they are non-technical and have
someone else taking over — help their new developer do it while the BlueWeb
project is still serving, then cut over. Do not delete the BlueWeb project
until their deploy answers on the domain.

### 3. Hand over the contact form

The BlueWeb Resend key must be revoked, and it will take the form down when it
is. Sequence it so the form is never silently broken:

1. They create their Resend account and verify the domain.
2. They set `RESEND_API_KEY`, `LEAD_TO`, `LEAD_FROM` on their own Pages
   project.
3. Test a submission on their deploy.
4. Only then revoke the BlueWeb key and delete the BlueWeb Pages project.

If they do not want a form at all, set `email: null` in `business.js` and it
disappears cleanly — tell them that, rather than leaving a dead form up.

### 4. Release the domain

Nameservers back to wherever they say, or to their new developer's setup. The
domain was always registered in their name; BlueWeb only ever held delegated
DNS. Remove the zone from BlueWeb's Cloudflare account once their nameservers
have moved.

### 5. Close out

- Remove BlueWeb collaborators from the transferred repo if they ask.
- Confirm in writing what has moved and what has been deleted.
- The footer credit is theirs to remove; `OFFLOAD.md` says so. Do not ask them
  to keep it.

## Verifying an offload actually worked

Not "the files are transferred" — **their** deploy, from **their** repo, on
**their** account, answering on the domain, with a test message arriving in
the owner's inbox. Check that before deleting anything of BlueWeb's.

```sh
curl -sI https://<their-domain> | grep -i -E 'content-security-policy|server'
```

## If BlueWeb disappears

The same invariant has to hold if BlueWeb stops existing rather than the
customer cancelling — which is the case `OFFLOAD.md` is actually written for.
Every customer has the file in their repo, so the instructions survive
independently of any BlueWeb system. That is the point of shipping it in the
repo rather than emailing it at the end.
