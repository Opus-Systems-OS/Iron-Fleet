// @ts-check
import { defineConfig } from 'astro/config';

// https://astro.build/config
export default defineConfig({
  // Canonical origin. Used for canonical URLs, sitemap and absolute links.
  // Until the customer's domain is pointed at Cloudflare this is still the
  // right value to ship — the *.pages.dev preview URL is noindexed by the
  // robots.txt rule, so nothing competes with the real domain in search.
  site: 'https://__DOMAIN__',

  build: {
    // /services/ rather than /services.html, so URLs stay clean with no
    // redirect rules to maintain.
    format: 'directory',

    // Never inline a stylesheet as a <style> element. public/_headers sets
    // style-src-elem 'self', which refuses inline <style> blocks — under the
    // default ('auto') a small enough stylesheet gets inlined and is then
    // silently blocked in the browser. External-always means that cannot
    // happen as the site grows.
    inlineStylesheets: 'never',
  },
});
