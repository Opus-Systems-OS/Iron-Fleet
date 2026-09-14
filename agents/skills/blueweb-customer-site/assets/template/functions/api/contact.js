/**
 * POST /api/contact — the contact form's inbox.
 *
 * This is a Cloudflare Pages Function: the file's path IS its route, it is
 * deployed by the same `git push` as the site, and it runs on the Workers
 * runtime at the same origin. That last part is why the CSP can stay at
 * `form-action 'self'` and `connect-src 'self'` — a third-party form service
 * would need both loosened, plus an account per customer.
 *
 * It stores nothing. The submission is turned into one email and forgotten,
 * so there is no database of other people's customers to secure, back up, or
 * hand over at the end of an engagement.
 *
 * Bindings (Cloudflare dashboard → the Pages project → Settings → Variables;
 * set them for BOTH Production and Preview or the preview form 500s):
 *
 *   RESEND_API_KEY   secret.  Resend API key, scoped to sending only.
 *   LEAD_TO          plain.   Where leads go — the owner's real inbox.
 *   LEAD_FROM        plain.   Verified Resend sender, e.g.
 *                             "Website <leads@customerdomain.com>".
 *
 * See references/backend-worker.md in the blueweb-customer-site skill for the
 * Resend domain-verification steps and the WAF rate-limit rule.
 */

/** Keep in sync with the honeypot input in src/components/ContactForm.astro. */
const HONEYPOT = 'hp_website';

const LIMITS = { name: 100, email: 200, phone: 40, message: 2000 };

const seeOther = (location) => new Response(null, { status: 303, headers: { location } });

/** Minimal, unstyled, no inline script — it has to survive the site's own CSP. */
function errorPage(status, headline, detail) {
  const esc = (s) => String(s).replace(/[<>&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;' }[c]));
  return new Response(
    `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">` +
      `<title>${esc(headline)}</title>` +
      `<body style="font:16px/1.6 system-ui,sans-serif;margin:0;padding:3rem 1.5rem;max-width:34rem">` +
      `<h1 style="font-size:1.5rem">${esc(headline)}</h1><p>${esc(detail)}</p>` +
      `<p><a href="/">Back to the site</a></p>`,
    { status, headers: { 'content-type': 'text/html; charset=utf-8' } },
  );
}

export async function onRequestPost({ request, env }) {
  const url = new URL(request.url);

  // Same-origin check. `form-action 'self'` already stops a compliant browser
  // from posting this form somewhere else; this stops someone else's page
  // from posting INTO it, which is the direction that actually costs the
  // customer — a form on a random domain spraying mail from their address.
  const origin = request.headers.get('origin');
  if (origin && new URL(origin).host !== url.host) {
    return errorPage(403, 'Blocked', 'That request did not come from this site.');
  }

  let form;
  try {
    form = await request.formData();
  } catch {
    return errorPage(400, 'Could not read that', 'The form data was malformed.');
  }

  const get = (k) => String(form.get(k) ?? '').trim();

  // Honeypot. Answer exactly as if it worked: a bot that gets an error learns
  // which field gave it away, and the next run leaves that field alone.
  if (get(HONEYPOT)) return seeOther('/thanks/');

  const lead = {
    name: get('name'),
    email: get('email'),
    phone: get('phone'),
    message: get('message'),
  };

  if (!lead.name || !lead.email || !lead.message) {
    return errorPage(400, 'Something was missing', 'Please fill in your name, email and message.');
  }
  for (const [field, max] of Object.entries(LIMITS)) {
    if (lead[field].length > max) {
      return errorPage(400, 'That was too long', `The ${field} field is limited to ${max} characters.`);
    }
  }
  // Deliberately loose. The only thing that matters is that the address can
  // be replied to, and a strict regex rejects valid addresses every year.
  if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(lead.email)) {
    return errorPage(400, 'Check that email address', 'That does not look like an email we could reply to.');
  }

  if (!env.RESEND_API_KEY || !env.LEAD_TO || !env.LEAD_FROM) {
    console.error('contact: missing binding —', {
      RESEND_API_KEY: Boolean(env.RESEND_API_KEY),
      LEAD_TO: Boolean(env.LEAD_TO),
      LEAD_FROM: Boolean(env.LEAD_FROM),
    });
    return errorPage(500, 'The form is not connected yet', 'Please call us instead — sorry about that.');
  }

  const body = [
    `Name:    ${lead.name}`,
    `Email:   ${lead.email}`,
    `Phone:   ${lead.phone || '—'}`,
    '',
    lead.message,
    '',
    `— sent from the contact form on ${url.host}`,
  ].join('\n');

  let res;
  try {
    res = await fetch('https://api.resend.com/emails', {
      method: 'POST',
      headers: {
        authorization: `Bearer ${env.RESEND_API_KEY}`,
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        from: env.LEAD_FROM,
        to: [env.LEAD_TO],
        // reply_to, not from: sending as the visitor's address would fail SPF
        // and land the whole domain in spam. Hitting reply in the owner's
        // inbox still goes to the visitor.
        reply_to: lead.email,
        subject: `New message from ${lead.name} — ${url.host}`,
        text: body,
      }),
    });
  } catch (err) {
    console.error('contact: fetch to Resend threw —', err);
    return errorPage(502, 'We could not send that', 'Please call us instead — sorry about that.');
  }

  if (!res.ok) {
    // Logged, not shown: the response can quote the API key's account details.
    // Read it with `npx wrangler pages deployment tail`.
    console.error('contact: Resend rejected —', res.status, await res.text());
    return errorPage(502, 'We could not send that', 'Please call us instead — sorry about that.');
  }

  return seeOther('/thanks/');
}

// Only onRequestPost is exported on purpose. Pages Functions answer 405 for a
// method with no matching handler, so GET /api/contact is already refused —
// and exporting a catch-all `onRequest` alongside it would shadow this one.
