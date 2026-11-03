import { selectorCatalog } from './sp-selector-data'
import { createRoot } from 'react-dom/client'
import { useState } from 'react'
import { TooltipProvider } from '../../../ade/web/src/components/ui/Tooltip'
import { PageShell, PageHeader, PageMain } from '@iii-dev/console-ui'
import { GroupsListView } from '../src/page/list'
import { useContainerNarrow } from '@iii-dev/console-ui/hooks'
import '../../../ade/web/src/index.css'
import '../styles.css'
import { client, type GroupSummary } from '../src/api'
const params = new URLSearchParams(location.search)
document.documentElement.dataset.theme = params.get('theme') || 'dark'
const now = 1793656200000
const groups = ['TimeoutError: upstream request exceeded deadline', 'ConnectionError: socket closed during response', 'TypeError: optional metadata missing', 'RuntimeError: unexpected worker exit', 'ParseError: malformed payload', 'NetworkError: name lookup failed'].map((title, i) => ({ id: `fixture-issue-${i + 1}`, fingerprint: `fixture-${i}`, source: 'trace', namespace: 'fixture', service_name: 'fixture-worker', function_id: 'fixture::read', exception_type: title.split(':')[0], title, status: 'new', occurrence_count: 7 + i, sessions_affected: 2, first_seen_ms: now - 7200000, last_seen_ms: now - 60000, sparkline: [0, 1, 0, 4, 2], has_diagnosis: false, relevant: true }) as GroupSummary)
const calls: unknown[] = []
const fixture = { calls, mode: params.get('mode') || 'success', delay: 200, baseline: '9eb9b507598a0dbe0937b883aebe89ebcba7f6bb' }
;(window as any).__SENTINEL_FIXTURE__ = fixture
const api = client({ call: async (id: string, payload: unknown) => {
  calls.push({ id, payload })
  if (id === 'directory::agents::list') {
    if (fixture.mode === 'catalog-error') throw new Error('Fixture catalog unavailable')
    if (fixture.mode === 'catalog-empty') return { agents: [] }
    if (params.get('selector') === 'long') return { agents: selectorCatalog }
    return { agents: [{ id: 'default', name: 'Default', description: 'Bundled profile fixture; real catalog wire shape' }, { id: 'broken-fixture', name: 'Broken chain', description: 'Unavailable', inheritance_error: 'missing parent' }] }
  }
  if (id === 'sentinel::investigate_many') {
    await new Promise(resolve => setTimeout(resolve, fixture.delay))
    if (fixture.mode === 'overlap') throw { code: 'sentinel/investigation_overlap', message: 'fixture-issue-1 overlaps with fixture-investigation-active; no partial start' }
    if (fixture.mode === 'transport-error') throw new Error('Fixture transport unavailable')
    return { investigation_id: 'fixture-investigation', session_id: 'fixture-session', existing: false, group_ids: (payload as any).group_ids, profile_id: (payload as any).profile_id }
  }
  throw new Error(`Unexpected isolated RPC: ${id}`)
} } as any)
function Fixture() {
  const { ref, narrow } = useContainerNarrow({ below: 760 })
  const [filters, setFilters] = useState({ statuses: ['new', 'regressed', 'investigating', 'diagnosed'] as any, window: '24h', service: '', search: '', relevance: 'relevant' })
  return <TooltipProvider><div ref={ref} data-iii-ui="sentinel" style={{ width: params.get('pane') ? `${params.get('pane')}px` : '100%', maxWidth: '100%', height: '100dvh' }}><PageShell data-narrow={narrow}><PageHeader title="Sentinel" description="Isolated fixture · no live engine" /><PageMain><div className="sentinel-ui-scroll"><GroupsListView api={api} onInvestigationStarted={result => calls.push({ started: result })} busy={false} onBulk={(...args) => calls.push(args)} status={null} filters={filters} groups={groups} loading={false} narrow={narrow} now={now} onFilters={setFilters} onOpen={(id) => calls.push(['open', id])} onMore={() => {}} total={groups.length} relevantTotal={groups.length} noiseTotal={0} workers={['fixture-worker']} /></div></PageMain></PageShell></div></TooltipProvider>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
