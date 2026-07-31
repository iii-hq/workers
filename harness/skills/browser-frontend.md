---
name: browser-frontend
description: >-
  Build and verify browser-facing React interfaces on iii. Use the browser
  worker for real page interaction, the browser SDK for browser-connected
  workers, and the console UI surface for native iii extensions.
---

# Browser and frontend architecture

Use the browser worker as the agent's browser boundary. A frontend is a real
web application or a console extension; it is not an HTTP-shaped substitute
for browser interaction.

## Choose the right boundary

- Use `browser::sessions::*`, `browser::navigate`, `browser::snapshot`, and
  `browser::act` to inspect and drive a running web page.
- Use the browser SDK when writing a browser-connected worker or client. Read
  the current reference at `https://iii.dev/docs/reference/sdk-browser.md`
  before writing the first SDK call.
- Use React for the UI itself. For an iii console extension, use the
  `@iii-dev/console-ui` components and the injectable console UI contract;
  keep React and the console package external in the asset build.
- Use a normal HTTP API only when the product explicitly needs an API. Do not
  turn the browser UI into a hand-built HTTP bridge just because browser
  functions were not visible in the current skill index.

## Build and verify

1. Discover the installed browser and console surfaces with
   `engine::functions::list` and fetch each contract with
   `engine::functions::info` before calling it.
2. Keep browser sessions explicit and short-lived: start one, navigate, act,
   inspect console/network evidence when something fails, and stop it when the
   check is complete.
3. Verify the UI through the browser worker against the running page. A
   successful HTTP response alone does not prove that the React application
   rendered or that its interactions work.
4. For a console extension, register a `console:script` or `console:style`
   asset through the worker and use the shared React components rather than
   bundling a second UI system.

The browser worker's full function and troubleshooting reference is available
as `browser/index`; this page exists so frontend architecture remains
available even before an optional browser worker is connected.
