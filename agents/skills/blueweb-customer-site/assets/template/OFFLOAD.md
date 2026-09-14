# Taking this site somewhere else

This file is for __BUSINESS_NAME__, or for whoever you hire next. It is
written on the assumption that BlueWeb is no longer involved and cannot be
asked questions.

**Everything the website *is* lives in this repository.** The pages, the
wording, the design, the contact form and its server code, the security
settings, and the automated checks. Nothing here is licensed to BlueWeb,
locked to BlueWeb, or built on a product only BlueWeb can buy.

There are exactly **three** things outside this repository, and each one has a
replacement you can set up yourself. None of them takes longer than an
afternoon.

---

## 1. The repository host — GitHub

Right now this repo lives in BlueWeb's GitHub organisation (`BlueWeb-Org`)
with you as a collaborator.

**To take it:** ask BlueWeb to transfer it to your own GitHub account or
organisation. GitHub calls this "Transfer ownership" and it keeps all the
history, branches and settings. If BlueWeb is unresponsive, you do not need
them — you can copy the whole thing instead:

```sh
git clone https://github.com/BlueWeb-Org/__SLUG__.git
cd __SLUG__
git remote set-url origin https://github.com/<you>/__SLUG__.git
git push -u origin main
```

That copy is complete. Nothing is left behind.

You do not have to use GitHub at all. This is an ordinary git repository and
works on GitLab, Bitbucket, or a hard drive.

---

## 2. The hosting — Cloudflare Pages

The site is currently published by a Cloudflare Pages project on BlueWeb's
Cloudflare account. The project holds no content: it reads this repository and
builds it. Recreating it on your own account takes about five minutes.

1. Make a free Cloudflare account.
2. Workers & Pages → Create → Pages → Connect to Git → pick your copy of this
   repo.
3. Settings:

   | Setting | Value |
   | --- | --- |
   | Production branch | `main` |
   | Build command | `npm run build` |
   | Build output directory | `dist` |
   | Root directory | *(leave empty)* |

4. Add your domain under the project's **Custom domains**.

Cloudflare is not required either. Any host that can run `npm ci && npm run
build` and serve the `dist/` folder will do — Vercel, Render, GitHub Pages,
Amazon S3, or a plain web server.

Two things to carry over if you move to a host that is not Cloudflare:

- **`public/_headers`** is the site's security configuration in Cloudflare's
  format. Netlify understands the same file. Anywhere else, translate it into
  that host's header configuration. The site works without it; it is simply
  less protected.
- **`functions/api/contact.js`** is the contact form's server code, in
  Cloudflare's format. See below.

---

## 3. The contact form's email — Resend

The form emails you through a service called Resend, using an API key on
BlueWeb's Resend account. **When BlueWeb's key stops working, the form stops
sending and the rest of the site is unaffected.**

To take it over:

1. Make your own free Resend account at <https://resend.com> (3,000 emails a
   month free, far more than this form will use).
2. Verify your domain there. Resend gives you DNS records to add; add them
   wherever your domain's DNS is managed.
3. Create an API key with sending permission.
4. In your Cloudflare Pages project → Settings → Variables and Secrets, set
   these for **both** Production and Preview:

   | Name | Kind | Value |
   | --- | --- | --- |
   | `RESEND_API_KEY` | Secret | your new key |
   | `LEAD_TO` | Plaintext | the inbox you want messages in |
   | `LEAD_FROM` | Plaintext | `Website <leads@yourdomain.com>` |

The form's code does not change. It reads whatever key it is given.

If you would rather not run a form at all, set `email: null` in
`src/data/business.js` and the form disappears from the site cleanly.

---

## Your domain

Your domain is registered in your name and always has been. If BlueWeb was
managing its DNS for you, that was delegated access, not ownership — sign in
to your registrar and point the nameservers wherever you want. BlueWeb cannot
prevent this and does not need to be involved.

---

## The BlueWeb credit

The footer says "Site by BlueWeb" and links to blueweb.ink. You are welcome to
remove it: it is one line in `src/components/Footer.astro`, and the `builder`
entry in `src/data/business.js`.

---

## What a new developer needs to know

Point them at `CLAUDE.md` in this repo. It is the technical working notes —
how to run it, how changes get published, and the handful of constraints that
are not obvious from reading the code. It was written to be handed over.

The stack is [Astro](https://astro.build), a widely used open-source static
site builder. There is no proprietary framework here and no BlueWeb library to
depend on.
