/**
 * The state-manager page (#/ext/state-manager): the standard page chrome
 * (PageShell/PageHeader from @iii-dev/console-ui) over the scopes × keys
 * browser (./browser) and its Monaco JSON value editor (./ValueEditor).
 *
 * One `state` trigger binding for the whole page (src/lib/events.ts),
 * fanned out to whichever views are mounted.
 */

import {
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  StatusDot,
} from '@iii-dev/console-ui'
import { Database } from 'lucide-react'
import { useStateEventHub } from '../lib/events'
import { StateBrowser } from './browser'

export function StateManagerPage({
  host,
  panelSide = 'left',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const subscribe = useStateEventHub(host)

  return (
    <PageShell className="state-ui-shell">
      <PageHeader
        icon={<Database size={16} />}
        title="State"
        description="Scoped key–value store, live"
        actions={
          <span
            className="state-ui-live"
            title="live — subscribed to the state trigger type; created/updated/deleted events stream in"
          >
            <StatusDot tone="accent" pulse /> live
          </span>
        }
        onClose={onRequestClose}
      />
      <StateBrowser
        host={host}
        subscribe={subscribe}
        panelSide={panelSide}
        panelContext={panelContext}
        commands={commands}
      />
    </PageShell>
  )
}
