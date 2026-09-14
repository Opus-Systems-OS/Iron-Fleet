# Intake

What to collect before writing anything. Every field here has a home in
`src/data/business.js`, in the same order.

## Ask for

**Identity.** Legal or trading name, and what they actually call themselves on
the sign. One line on what they do — in their words, then tightened. Not
"Your trusted partner in automotive excellence".

**Contact.** Phone number, in the format they say it out loud. Whether they
want an email address on the site at all — plenty of owners do not want one,
and a form with nowhere to go is worse than no form.

**Location.** Full street address, or a service area if they work out of a
van. `hasStorefront: false` drops the address block and keeps the hours.

**Hours.** Including the days they are closed, and anything conditional
("Saturdays by appointment", "closed the first week of August"). Conditional
text goes in `hoursNote`, never in the table — the table also feeds the
structured data that Google shows in search.

**Services.** Four to eight. Each one a name, one or two plain sentences, and
a price *if they want prices on the site*. Ask that question explicitly rather
than assuming: some owners quote per job and a public price costs them room to
negotiate. Leave `price: null` and the card renders cleanly without one.

**Proof.** Years in business, licences, certifications, family ownership,
anything on the wall behind the counter. These become the line above the
headline and they are the highest-converting text on the page.

**Social.** Instagram, Facebook, Yelp, Google Business. Whichever exist. Only
links that already have real content on them — an empty Facebook page loses
more trust than a missing one.

**Colors.** Photograph their sign, their truck, their business card. Matching
what a customer already recognises beats a nicer palette. The accent color has
to pass 4.5:1 against white for body text; if theirs is too light, keep it for
large headings and darken the accent.

**Photos.** Ask for real ones of the shop, the team and the work — an owner
with 400 job photos on their phone is common. Stock photography of a generic
storefront reads as fake immediately.

## Answers that need a follow-up

- **"Whatever you think is best"** on hours, prices or services. Get the facts
  anyway; guessing at a competitor's hours and putting them on Google is
  worse than an empty section.
- **"I'll send you the logo."** They usually will not before the demo. The
  scaffold ships a placeholder favicon with their initial — that is fine to
  demo with, but chase the real one before launch.
- **No domain yet.** Scaffold with the domain they intend to buy. It only
  affects canonical URLs, and the whole site is fine before it exists. Domain
  purchase is the customer's, in every plan — see `handoff.md`.
- **"Can people book online?"** Slow down. Read `backend.md` before agreeing
  to anything with accounts, payments or a calendar.

## Before you start building

You should be able to fill in every non-`OPTIONAL` field in `business.js`
without guessing. If you cannot, the intake is not finished — the gap will
otherwise turn into placeholder text that survives to the demo.
