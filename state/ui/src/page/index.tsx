/**
 * The state-manager page (#/ext/state-manager): the standard page chrome
 * (PageShell/PageHeader from @iii-dev/console-ui) over the scopes × keys
 * browser (./browser) and its Monaco JSON value editor (./ValueEditor).
 *
 * One `state` trigger binding for the whole page (src/lib/events.ts),
 * fanned out to whichever views are mounted.
 */

import { type Host, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { useStateEventHub } from '../lib/events'
import { DatabaseIcon, LiveDot } from '../lib/widgets'
import { StateBrowser } from './browser'

export function StateManagerPage({
  host,
  panelSide = 'left',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const subscribe = useStateEventHub(host)

  return (
    <PageShell className="state-ui-shell">
      <PageHeader
        icon={<DatabaseIcon />}
        title="state"
        description="scoped key–value store, live"
        actions={<LiveDot />}
        onClose={onRequestClose}
      />
      <StateBrowser host={host} subscribe={subscribe} panelSide={panelSide} />
    </PageShell>
  )
}
