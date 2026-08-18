/**
 * Console surface for the discovery worker: the injected call card for
 * `discovery::search_functions` results, plus a null transcript renderer
 * that keeps the hook's `origin.discovery` annotations out of chat.
 */

import type { Host } from '@iii-dev/console-ui'

import { createSearchTriggerRenderer } from './src/search-card'
import { DiscoveryConfigForm } from './src/config-form'

/**
 * The hook's transcript annotations stay in the durable data (for traces
 * and measurement) but render nothing in chat: registering a null renderer
 * is what SUPPRESSES the console's fallback summary row — without a
 * registration every generation would print "discovery · hint injected"/
 * "discovery · skipped" lines.
 */
function DiscoveryPassLine() {
  return null
}

export default function setup(host: Host) {
  host.functionTriggers.register(createSearchTriggerRenderer())
  host.configForms.register('discovery', (props) => (
    <DiscoveryConfigForm {...props} host={host} />
  ))
  host.chat?.registerTranscriptRenderer?.({
    id: 'discovery',
    render: DiscoveryPassLine,
  })
}
