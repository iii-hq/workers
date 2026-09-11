/**
 * Injected renderer for agent-proposed working-directory changes. The
 * function only validates and stamps a proposal; this UI owns the explicit
 * operator confirmation that applies it to the matching Console session.
 */

import type { Host } from '@iii-dev/console-ui'
import { createWorkingDirectoryProposalRenderer } from './src/working-directory'

export default function setup(host: Host) {
  host.functionTriggers.register(createWorkingDirectoryProposalRenderer(host))
}
