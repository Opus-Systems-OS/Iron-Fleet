# __BUSINESS_NAME__

The website for __BUSINESS_NAME__, at [__DOMAIN__](https://__DOMAIN__).

This repository holds everything the site is made of. It is yours — the text,
the images, the code. If you ever stop working with BlueWeb, you can hand this
repository to anyone else and they can pick it up.

## Where the important things are

| What | Where |
| --- | --- |
| Your name, phone, address, hours, services and prices | `src/data/business.js` |
| The wording on the homepage | `src/components/` and `src/pages/index.astro` |
| Photos and your logo | `public/` |
| Where contact-form messages get emailed | Cloudflare dashboard → Variables |

Almost everything you would ever want changed lives in that first file.

## Asking for a change

Text or email BlueWeb. Changes go live within three business days on the
maintenance plan. You do not need to touch anything in here yourself — but if
you want to, nothing stops you.

## If you ever want to leave

Read [OFFLOAD.md](OFFLOAD.md). It explains how to take this site, your domain
and your contact form somewhere else entirely, step by step, without needing
BlueWeb's help or permission. Nothing here is locked to us.

## Running it on your own computer

You need [Node.js](https://nodejs.org) 22 or newer.

```sh
npm ci
npm run dev
```

Then open <http://localhost:4321>. Edits show up as you save.

## How it gets published

Pushing to the `main` branch publishes the site. Cloudflare Pages builds it
and puts it live at __DOMAIN__, usually within a minute. Every proposed change
gets its own preview link first, so nothing goes live unseen.

---

Built by [BlueWeb](https://blueweb.ink).
