---
title: web-performance
description: Ship fast interfaces — Core Web Vitals and field measurement, performance budgets, loading and code-splitting strategy, image and font cost, main-thread work and INP, layout stability, and caching.
type: how-to
---

# Web performance

**Measure before optimizing, and measure in the field.** A Lighthouse score is a lab proxy; the
number that matters is the 75th percentile of real users.

## The metrics, and what each one is really about

| Metric | Meaning | Dominated by |
| --- | --- | --- |
| **LCP** | when the main content appears | server/TTFB, render-blocking CSS, the hero image, fonts |
| **INP** | responsiveness of interactions | main-thread work in event handlers and renders |
| **CLS** | visual stability | unsized media, late-injected banners/ads, web fonts, layout-shifting hydration |
| **TTFB** | server responsiveness | caching, CDN, backend work (a browser cannot fix this) |

Targets: LCP ≤ 2.5 s, INP ≤ 200 ms, CLS ≤ 0.1 — at p75.

- **Lab:** Chrome DevTools Performance panel (with CPU/network throttling), Lighthouse,
  PageSpeed Insights. Use them to explain a problem, not to certify a fix.
- **Field:** the `web-vitals` package (`onLCP`, `onINP`, `onCLS`) reporting to your own endpoint,
  or the CrUX / Vercel Speed Insights data. Ship this before optimizing anything.
- **Budgets, enforced:** a max JS KB per route and per metric, checked in CI
  (`size-limit`, `lighthouse-ci`). An unenforced budget decays within a quarter.

## Loading: get less code there sooner

- **Route-level code splitting is the highest-leverage change.** `React.lazy` /
  `createLazyFileRoute` per route; the entry chunk should be shell + router, nothing else.
- **Do not create waterfalls.** Sequential `await import()` calls cost a round trip each; issue
  independent imports in parallel, or fetch in the route loader so data and chunks load together.
- `modulepreload` the chunks a route needs; keep `<link rel="preload">` for the current
  navigation only, since unused preloads compete for bandwidth.
- **Audit every dependency by cost.** `date-fns` over `moment`, `lucide-react` (tree-shakeable)
  over an icon bundle, `Intl` over a locale bundle. Check the actual bytes in the bundle
  analyzer — "it's tree-shakeable" is a claim, not a measurement.
- Defer anything not needed for first paint: analytics, chat widgets, devtools, editors.
- Inline critical CSS (or let the framework do it) and avoid render-blocking third-party CSS.
- Preconnect to origins you will definitely use (font, API, CDN) — and no more than a few.

## Images and fonts: usually the largest bytes

- Correct format: AVIF/WebP with a fallback; correct dimensions; never ship a 2000 px image into
  a 400 px slot.
- `srcset` + `sizes` for fluid images; `width` and `height` (or `aspect-ratio`) on every image so
  the browser reserves space — this is most of CLS.
- `loading="lazy"` and `decoding="async"` below the fold; **never lazy-load the LCP image**.
  Give the hero `fetchpriority="high"`.
- Fonts: self-host, subset to the characters you use, `font-display: swap`, preload the one
  weight that paints first, and set `size-adjust`/`ascent-override` on a fallback face to keep
  the swap from shifting layout.
- Use an image CDN with width/quality parameters rather than committing many variants.

## Main thread and INP

INP is not about the network; it is about how long the browser is busy.

- **Keep event handlers under ~50 ms of work.** Yield with `scheduler.yield()` (or
  `setTimeout`) inside loops that must run long.
- Move heavy computation off the render path: `useDeferredValue` / `startTransition` for
  expensive re-renders, a Web Worker for pure CPU work, the server for anything that can be.
- **Virtualize long lists.** Mounting 2000 rows is a main-thread cost paid by every user.
- Avoid layout thrash: batch DOM reads, then writes; never interleave
  `getBoundingClientRect()` with style writes.
- Debounce/throttle scroll, resize, and input handlers; prefer `ResizeObserver`/
  `IntersectionObserver` over polling.
- Watch for third-party scripts executing on the main thread; they are invisible in your bundle
  report and very visible in INP.
- Long Animation Frames (`PerformanceObserver` with `long-animation-frame`) attribute slowness to
  a script URL — far more actionable than a total blocking time number.

## CLS specifics

- Reserve space for anything that loads late: images, ads, embeds, banners, toasts.
- Never insert a bar above existing content after it has rendered.
- Animations must use `transform` and `opacity` only.
- Content-visibility and `contain` prevent out-of-viewport painting from affecting layout.

## Caching and delivery

- Hashed assets: `Cache-Control: public, max-age=31536000, immutable`.
- HTML: short max-age or `no-cache`, with `stale-while-revalidate` at the CDN.
- Compress (brotli/gzip) and serve from a CDN close to the user; TTFB caps everything else.
- Do not cache a page that embeds per-user data without varying on the right key.

## Workflow

1. Get the field numbers (or, absent them, a throttled lab trace of the real flow).
2. Identify the **one** dominant cost. Usually: too much JS, an unoptimized hero image, an
   oversized font, a blocking third-party script, or a chatty waterfall.
3. Fix it, and re-measure the same way. A change you did not measure is a change you do not know.
4. Record the budget and add the CI check, so it cannot silently regress.

## The short list to refuse

- Optimizing without a measurement, or "optimizing" the bundle you assumed was the problem.
- Shipping a new heavy dependency without checking its bytes.
- Lazy-loading the LCP image, or lazy-loading everything as a habit.
- Unsized images and late banners (free CLS).
- Long synchronous work in a click handler.
- Third-party scripts loaded synchronously in `<head>`.
- A Lighthouse score as the goal instead of p75 field metrics.
