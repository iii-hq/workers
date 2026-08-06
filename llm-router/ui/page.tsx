/**
 * llm-router's injectable console UI: a custom configuration form for the
 * `llm-router` entry (provider credentials, routing heuristics, stream
 * budgets) with plain-text-secret detection on api keys. Registered per
 * the injectable-ui contract (docs/sops/injectable-console-ui.md).
 */

import type { Host } from '@iii-dev/console-ui'
import { LlmRouterConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('llm-router', LlmRouterConfigForm)
}
