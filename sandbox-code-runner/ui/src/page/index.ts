/**
 * The sandbox fleet page module — everything page.tsx registers:
 *
 * - `SandboxPage`             → `host.pages.register` (#/ext/sandbox)
 * - `SandboxConfigForm`       → `host.configForms.register('sandbox-code-runner', …)`
 * - `createSandboxSessionChip`→ `host.chat?.registerSessionChip` (feature-
 *                               detected: older consoles have no chat slot)
 *
 * The stylesheet lives at src/styles/page.css (`cr-page-*` rules under the
 * worker's `[data-iii-ui="sandbox-code-runner"]` scope).
 */

export { SandboxConfigForm } from './ConfigForm'
export { createSandboxSessionChip } from './chip'
export { SandboxPage } from './SandboxPage'
