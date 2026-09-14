/**
 * THE INTAKE FILE.
 *
 * Everything the site says about the business lives here — nothing else in
 * src/ should hardcode a name, phone number, price or opening time. When the
 * owner texts "we open at 8 now", this is the only file that changes, and
 * that is the whole reason the $99/month plan is a three-day promise rather
 * than a project.
 *
 * Fields marked OPTIONAL can be left null/empty and the components that use
 * them disappear cleanly rather than rendering an empty box. Do not delete
 * the key — set it to null so the next person knows the option exists.
 */

export const business = {
  name: '__BUSINESS_NAME__',
  // Short form for the nav and tight spaces. Falls back to `name`.
  shortName: null,
  // One line, plain English, no marketing voice. This is the <title> suffix
  // and the meta description seed. Say what they do and where.
  tagline: 'TODO — what they do, in one line',
  domain: '__DOMAIN__',

  // --- how customers reach them ------------------------------------------
  // `phone` is the raw display string; `phoneHref` is the tel: value.
  // Local businesses convert on calls far more than on forms — the phone
  // number is the primary call to action on every page.
  phone: 'TODO',
  phoneHref: 'tel:+1TODO',
  // OPTIONAL. Where the contact form's lead email is delivered.
  // Null means the site ships with no contact form at all (see functions/).
  email: null,
  // OPTIONAL. Set to false for a mobile/at-home business with no storefront
  // — the address block and map link drop out and the hours stay.
  hasStorefront: true,
  address: {
    street: 'TODO',
    city: 'TODO',
    state: 'CA',
    zip: 'TODO',
  },
  // OPTIONAL. Google Maps place link. A search URL built from the address
  // is a fine substitute, but a real place link shows their reviews.
  mapUrl: null,

  // --- hours --------------------------------------------------------------
  // `closed: true` renders "Closed" for that day. Days render in this order,
  // so leave it Monday-first unless the owner asks otherwise.
  hours: [
    { day: 'Monday', open: '9:00am', close: '6:00pm' },
    { day: 'Tuesday', open: '9:00am', close: '6:00pm' },
    { day: 'Wednesday', open: '9:00am', close: '6:00pm' },
    { day: 'Thursday', open: '9:00am', close: '6:00pm' },
    { day: 'Friday', open: '9:00am', close: '6:00pm' },
    { day: 'Saturday', open: '10:00am', close: '4:00pm' },
    { day: 'Sunday', closed: true },
  ],
  // OPTIONAL. Free text under the hours table — holiday closures, "walk-ins
  // until 5", "by appointment only".
  hoursNote: null,

  // --- what they sell -----------------------------------------------------
  // `price` is OPTIONAL per service. Owners who do not want prices on the
  // site say so in intake; leave it null and the card renders without one
  // rather than saying "Call for pricing", which reads as evasive.
  services: [
    { name: 'TODO', price: null, body: 'TODO — one or two plain sentences.' },
  ],
  // OPTIONAL. Shown above the services as a single trust line: "Family owned
  // since 1998", "Licensed and insured", "Same-day service".
  proofPoints: [],

  // --- social -------------------------------------------------------------
  // OPTIONAL, all of them. Only the ones with a value render an icon.
  social: {
    instagram: null,
    facebook: null,
    yelp: null,
    google: null,
  },

  // --- look ---------------------------------------------------------------
  // Two colors drive the whole palette (see src/styles/global.css). Pull them
  // from the customer's existing sign, truck or logo — matching what is
  // already on their storefront is worth more than a "nicer" color.
  // `accent` must pass 4.5:1 against white for body-size text; if the brand
  // color is too light, keep it for large headings only and darken this one.
  brand: {
    accent: '#0b5cad',
    ink: '#12181f',
  },
};

/**
 * Delivered by BlueWeb. Rendered as a small line in the footer.
 * Keep it — it is the referral channel, and every customer agreed to it.
 */
export const builder = {
  name: 'BlueWeb',
  url: 'https://blueweb.ink',
};

/** `shortName` with a fallback, so components never have to think about it. */
export const displayName = business.shortName || business.name;
