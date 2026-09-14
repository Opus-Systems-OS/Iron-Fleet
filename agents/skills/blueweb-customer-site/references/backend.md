# Backends

**The default is no backend.** A local business site is a phone number, hours,
and enough proof to make someone call. Most customer sites should ship as
static HTML with nothing running behind them. Every backend added is something
that can break on a Saturday for a business that cannot fix it.

The ladder below is in order. Do not skip a rung because the customer asked
for the top one.

## Rung 0 — phone and address only

`business.email: null` and the contact form does not render. Nothing to
deploy, nothing to secure, nothing to bill. For a barber, a detailer or a
mobile mechanic this is frequently the right and final answer.

## Rung 1 — the contact form (in the template)

`functions/api/contact.js`, a Cloudflare Pages Function at the same origin as
the site. One form submission becomes one email and nothing is stored.

Why same-origin rather than Formspree or Web3Forms: the CSP keeps
`form-action 'self'` and `connect-src 'self'`, there is no per-customer
third-party account to pay for or lose access to, and no one else's customers
end up in a vendor's database.

### Wiring it up

Cloudflare dashboard → the Pages project → Settings → Variables and Secrets.
**Set all three for Production *and* Preview** — a form that works in
production and 500s on every PR preview means the bindings were only set on
one environment.

| Name | Kind | Value |
| --- | --- | --- |
| `RESEND_API_KEY` | Secret | Resend key, sending permission only |
| `LEAD_TO` | Plaintext | The owner's real inbox |
| `LEAD_FROM` | Plaintext | `Website <leads@customerdomain.com>` |

### Resend domain verification

`LEAD_FROM` must be on a domain verified in Resend, which means adding the
DKIM and SPF records Resend gives you to the customer's DNS. Since DNS is
already on Cloudflare for the custom domain, this is a few records in the same
dashboard. Do it when the domain is set up, not on demo day — propagation is
usually minutes but is not instant.

Mail from an unverified domain either fails outright or lands in spam, and a
contact form that silently spams is worse than no form.

`reply_to` carries the visitor's address; `from` stays the customer's domain.
Sending *as* the visitor fails SPF and takes the whole domain's reputation
down with it.

### Testing it

Submit the form on the Cloudflare preview URL, not localhost — the Function
does not run under `astro dev`. Watch `npx wrangler pages deployment tail` if
nothing arrives. Check the honeypot too: a submission with `hp_website` filled
must return the same 303 to `/thanks/` as a real one.

### If spam gets through

The honeypot stops the bulk of it. In order, when it does not:

1. **A Cloudflare WAF rate-limiting rule** on `/api/contact` — e.g. 5 requests
   per 10 minutes per IP. Free, no code, no visitor-facing change.
2. **Cloudflare Turnstile.** Real work: a widget script the CSP must allow,
   plus server-side verification in the Function. Only worth it under
   sustained abuse.

Do not add a captcha pre-emptively. It costs real submissions from exactly the
customers a local business wants.

## Rung 2 — something that stores data

Bookings, quote requests with a status, a members area. This is a different
product with a different price, and it changes the ongoing relationship: the
customer now has data that must be backed up, secured, and handed over.

Before building anything, check whether the customer already has a tool that
does it. Most trades already run Jobber, Housecall Pro or Square Appointments;
most salons run Booksy or Fresha. A link to a booking page they already pay
for is better than a bespoke calendar that nobody maintains.

If it genuinely has to be built:

- **Cloudflare D1** for relational data. Same account, same deploy, bound to
  the Function.
- **Cloudflare KV** only for caches and rate-limit counters, never for the
  record of a customer's booking.
- **R2** for uploads. Never let visitors write to it unmediated.
- Adding a database means adding backups and a retention answer. Write both
  down in the repo's `CLAUDE.md` at the time, not later.

## Rung 3 — payments, accounts, anything regulated

Stop and talk to the customer about scope and price. Taking card details, or
holding anything covered by health or employment rules, is not a $700 site.
Never build a payment flow into a brochure site — link to Stripe, Square or
whatever they already use.
