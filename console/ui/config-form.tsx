/**
 * Entry for the console worker's own injected UI — compiled by esbuild
 * (react + @iii-dev/console-ui external) into dist/config-form.js and
 * served over the `console:script` trigger (see src/ui.rs). The stylesheet
 * is its own asset: styles.css ships over `console:style` as
 * console/styles.css.
 *
 * Contributions: the custom form for the `console` entry and an explicit,
 * declarative form for every configurable worker that does not ship its own
 * override. Registered through `host` so the loader disposes every form on
 * hot reload / disconnect.
 */

import type { ConfigFormProps, Host } from '@iii-dev/console-ui'
import {
  configurationForm,
  workerConfigurationIds,
  workerConfigurationManifest,
  workerConfigurationSpecs,
} from './src/configuration-forms'
import { InjectableUiConfigForm } from './src/injectable-ui-form'

export { workerConfigurationIds, workerConfigurationManifest, workerConfigurationSpecs }

export default function setup(host: Host) {
  host.configForms.register('console', (props: ConfigFormProps) => <InjectableUiConfigForm host={host} {...props} />)

  host.configForms.register('a2ui', configurationForm('a2ui'))
  host.configForms.register('approval-gate', configurationForm('approval-gate'))
  host.configForms.register('bridge', configurationForm('bridge'))
  host.configForms.register('canvas', configurationForm('canvas'))
  host.configForms.register('claude-code', configurationForm('claude-code'))
  host.configForms.register('codex', configurationForm('codex'))
  host.configForms.register('computer', configurationForm('computer'))
  host.configForms.register('cursor', configurationForm('cursor'))
  host.configForms.register('devin', configurationForm('devin'))
  host.configForms.register('document', configurationForm('document'))
  host.configForms.register('editor', configurationForm('editor'))
  host.configForms.register('email', configurationForm('email'))
  host.configForms.register('fp', configurationForm('fp'))
  host.configForms.register('github', configurationForm('github'))
  host.configForms.register('grok', configurationForm('grok'))
  host.configForms.register('harness', configurationForm('harness'))
  host.configForms.register('http', configurationForm('http'))
  host.configForms.register('iii-observability', configurationForm('iii-observability'))
  host.configForms.register('memory', configurationForm('memory'))
  host.configForms.register('memory-consolidate', configurationForm('memory-consolidate'))
  host.configForms.register('opencode', configurationForm('opencode'))
  host.configForms.register('openwiki', configurationForm('openwiki'))
  host.configForms.register('pdf', configurationForm('pdf'))
  host.configForms.register('pi', configurationForm('pi'))
  host.configForms.register('provider-xai', configurationForm('provider-xai'))
  host.configForms.register('pubsub', configurationForm('pubsub'))
  host.configForms.register('queue', configurationForm('queue'))
  host.configForms.register('rbac-proxy', configurationForm('rbac-proxy'))
  host.configForms.register('sandbox-code-runner', configurationForm('sandbox-code-runner'))
  host.configForms.register('scrapling', configurationForm('scrapling'))
  host.configForms.register('security-scan', configurationForm('security-scan'))
  host.configForms.register('session-manager', configurationForm('session-manager'))
  host.configForms.register('shell', configurationForm('shell'))
  host.configForms.register('slack', configurationForm('slack'))
  host.configForms.register('tailscale', configurationForm('tailscale'))
  host.configForms.register('telegram-bot', configurationForm('telegram-bot'))
  host.configForms.register('vscode', configurationForm('vscode'))
  host.configForms.register('web', configurationForm('web'))
  host.configForms.register('workflow', configurationForm('workflow'))
  host.configForms.register('worktree', configurationForm('worktree'))
}
