---
name: ade-for-mcp-agents
description: >-
  Rules for agents that reach the engine through mcp-server (Claude Code and
  other MCP clients) when they build something people use: pages go into the
  ADE as injectable UI, apps call functions from the browser instead of HTTP
  endpoints, and the user gets the page's URL. Preloaded by the default-mcp
  profile.
---

# The ADE, for MCP agents

How to build what people use in an iii project when you reach the engine
through mcp-server. Follow these unless the user asks for something else.

## Pages are injectable UI in the ADE

- Build a page for a worker or a service as an injectable page in the ADE: a
  script the worker registers and the ADE loads (`setup(host)`,
  `host.pages.register`). Do not build a separate web server or static site
  for it. Read the `ade/injectable-ui` skill before building, and
  `ade/design-system` for how it looks.
- If the ADE is not installed (`ade::status` answers `function_not_found`),
  install it with `compose::add` first (worker `ade`).

## APIs are functions, called from the browser

Unless the API is public, for third parties, do not serve HTTP endpoints
through the `http` worker. Register functions, and call them from the browser:

- an injected ADE page uses `host.iii`, the tab's connection to the engine
  (`call`, `on`, `observe`);
- any other browser app uses the iii browser SDK (`iii-browser-sdk` on npm).

Use the `http` worker only for an API given to third parties, including
endpoints they call, such as webhooks.

## Give the user the page's URL

Once the page is registered, give the user its URL:
`<url>/#/worker/<scope>/<page-id>`, with `url` from `ade::status`
(`http://127.0.0.1:3113` by default; read it, never assume the port) and
`<scope>` and `<page-id>` as the `ade/injectable-ui` skill describes them. For
example: `http://127.0.0.1:3113/#/worker/mywork/mywork-manager`. Check that
`ade::ui-manifest` lists the page before you report it.
