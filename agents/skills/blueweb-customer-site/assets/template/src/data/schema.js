/**
 * LocalBusiness structured data.
 *
 * For a local business this is the highest-value markup on the site: it feeds
 * the Google knowledge panel and the map pack, which is where the customer's
 * actual walk-ins come from.
 *
 * It is built here rather than inline in the layout so that the hours Google
 * shows and the hours in the page's own table can never disagree — both read
 * the same `business` object.
 *
 * This is the site's only inline <script>, and it deliberately has no CSP hash
 * in public/_headers: browsers do not apply script-src to
 * `type="application/ld+json"`, which is data and never executed. See the
 * comment in public/_headers for how that was verified.
 */

/** '9:00am' -> '09:00'. Schema.org wants 24-hour ISO times. */
function to24h(t) {
  const m = /^(\d{1,2}):(\d{2})\s*(am|pm)$/i.exec(String(t).trim());
  if (!m) throw new Error(`business.hours: cannot parse time "${t}" (expected "9:00am")`);
  let h = Number(m[1]) % 12;
  if (m[3].toLowerCase() === 'pm') h += 12;
  return `${String(h).padStart(2, '0')}:${m[2]}`;
}

export function localBusinessJsonLd(business) {
  const url = `https://${business.domain}`;

  const data = {
    '@context': 'https://schema.org',
    // Swap for a more specific type where one exists — HairSalon, AutoRepair,
    // NailSalon, Gym, Restaurant. Google treats the specific type as a
    // stronger signal than the generic one.
    '@type': 'LocalBusiness',
    name: business.name,
    description: business.tagline,
    url,
    telephone: business.phone,
  };

  if (business.email) data.email = business.email;

  if (business.hasStorefront) {
    data.address = {
      '@type': 'PostalAddress',
      streetAddress: business.address.street,
      addressLocality: business.address.city,
      addressRegion: business.address.state,
      postalCode: business.address.zip,
      addressCountry: 'US',
    };
  }

  const open = business.hours.filter((h) => !h.closed);
  if (open.length) {
    data.openingHoursSpecification = open.map((h) => ({
      '@type': 'OpeningHoursSpecification',
      dayOfWeek: `https://schema.org/${h.day}`,
      opens: to24h(h.open),
      closes: to24h(h.close),
    }));
  }

  const sameAs = Object.values(business.social || {}).filter(Boolean);
  if (sameAs.length) data.sameAs = sameAs;

  return JSON.stringify(data, null, 2);
}
