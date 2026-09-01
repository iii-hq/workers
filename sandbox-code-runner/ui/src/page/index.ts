/**
 * The sandbox fleet page module — everything page.tsx registers:
 *
 * - `SandboxPage`             → `host.pages.register` (#/ext/sandbox)
 * - `createSandboxSessionChip`→ `host.chat?.registerSessionChip` (feature-
 *                               detected: older consoles have no chat slot)
 *
 * The `sandbox-code-runner` configuration entry uses the explicit form
 * registered in global Settings (its one knob is `inject_guidance`; the
 * operator FILE config — timeouts, idle TTL — never lived in that entry).
 *
 * The stylesheet lives at src/styles/page.css (`cr-page-*` rules under the
 * worker's `[data-iii-ui="sandbox-code-runner"]` scope).
 */

export { createSandboxSessionChip } from './chip'
export { SandboxPage } from './SandboxPage'
