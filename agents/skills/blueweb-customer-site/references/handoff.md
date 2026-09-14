# Handoff

BlueWeb builds the whole site, shows it in person, and gets paid only if the
owner says yes. So handoff is a real moment, not a formality — and repo access
is part of what is being handed over.

## Timing

Create the repo private under `BlueWeb-Org` while building. **Invite the customer
at handoff, after payment**, unless they ask earlier — in which case just do
it, the site is going to be theirs regardless.

## Giving access

```sh
gh api -X PUT repos/BlueWeb-Org/<slug>/collaborators/<their-github-username> \
  -f permission=push
```

They get an email invitation they must accept. `push` rather than `pull`: it
is their site, and an owner who wants to fix a typo themselves should be able
to. `admin` stays with BlueWeb while BlueWeb is maintaining it, so nobody
accidentally deletes the repo or disconnects the Cloudflare build.

Most owners do not have a GitHub account. Do not make one for them — that is
an account in their name with a password neither of you should hold. Either
walk them through signing up at the handoff meeting, or note in the handover
that access is available whenever they want it and move on. The `README.md` in
the repo is written for them, not for developers, so it makes sense whenever
they do get there.

## What changes by plan

| | $500 files only | $700 launched | $700 + $99/month |
| --- | --- | --- | --- |
| Repo access | Yes, at handoff | Yes, at handoff | Yes, at handoff |
| Deployed by BlueWeb | No | Yes | Yes |
| Cloudflare project | Theirs to create | BlueWeb's account | BlueWeb's account |
| Domain owner | Customer | Customer | Customer |
| DNS managed by | Customer | Customer | BlueWeb (delegated) |
| Ongoing changes | No | No | Within 3 business days |

**The domain is the customer's in every column.** They register it, they own
it, they can take it elsewhere. On the maintenance plan BlueWeb holds
delegated DNS, not ownership.

### $500, files only

They get the repo and take it from there. Include the deployment instructions
— `deploy-cloudflare.md`'s "Create the project" table is enough for anyone
technical — and be clear that support ends at the handover. Do not create a
Cloudflare project on the BlueWeb account for a site BlueWeb is not launching.

### $700, built and launched

BlueWeb creates the Pages project and points the customer's domain at it.
After launch the site is theirs to run. If they later want to move it off the
BlueWeb Cloudflare account, they create their own project against the same
repo and repoint the nameservers — nothing in the repo has to change. Say that
out loud; it is the thing that makes "you own it" credible.

### $700 + $99/month

BlueWeb keeps admin on the repo and the Cloudflare project and manages DNS.
Change requests go through the normal branch → PR → preview → merge flow. If a
request cannot be handled by editing `business.js`, add the field to
`business.js` rather than hardcoding it — that is what keeps a three-day
promise cheap.

## Walk them through

Five minutes, in person, on their phone:

- The live site on their own phone, and the phone-call button working.
- Their Google listing showing the right hours (the structured data feeds it).
- Where to text a change request, and what three business days means.
- That the domain is in their name and the repo is theirs, and that
  `OFFLOAD.md` in the repo tells them how to take the whole thing
  elsewhere without asking BlueWeb.
- The contact form, if they have one — send a test message and let them watch
  it land in their inbox.

## Cancellation and offload

Stopping the monthly does not take the site down, and every piece of BlueWeb
copy says so. Practically: the Cloudflare project keeps serving the last
deploy indefinitely and nothing expires. What stops is BlueWeb making changes.

If they want to leave entirely, that is a supported, documented path rather
than a negotiation — the repo ships `OFFLOAD.md` for exactly this, written for
the customer and whoever they hire next. See `offload.md` for the runbook and
for the portability rule that keeps it true as sites grow.

Say this at handoff, unprompted. "You can take this anywhere, and here is the
file that tells you how" is the most persuasive thing about the $99/month
plan, because it makes the monthly a choice rather than a leash.
