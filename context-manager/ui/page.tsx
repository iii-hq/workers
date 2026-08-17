/**
 * Context-manager's injected console contribution. The host owns dirty
 * tracking, validation, save, and reset; this module replaces only the
 * generic configuration fields with an operator-focused layout.
 */

import type { Host } from '@iii-dev/console-ui'
import { ContextManagerConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('context-manager', ContextManagerConfigForm)
}
