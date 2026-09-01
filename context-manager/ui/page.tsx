/**
 * Context-manager's injected console contribution. The host owns dirty
 * tracking, validation, save, and reset; this module provides the
 * operator-focused form body.
 */

import type { Host } from '@iii-dev/console-ui'
import { ContextManagerConfigForm } from './src/configuration'

export default function setup(host: Host) {
  host.configForms.register('context-manager', ContextManagerConfigForm)
}
