import { type Host, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { RefreshCcw } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { createEvalApi } from '../api'
import { parseOpenContext } from '../model'
import { CycleView, type OpenRequest } from './CycleView'

export function EvalPage({
  host,
  panelSide = 'left',
  tabId = '',
  paneId,
  panelContext,
  onRequestClose,
  setDirty,
}: { host: Host } & Partial<PageRenderProps>) {
  const api = useMemo(() => createEvalApi(host), [host])
  const [openRequest, setOpenRequest] = useState<OpenRequest | null>(null)
  const handled = useRef(-1)

  // `host.panels.open` from the palette or a command: a cycle, the start form or the settings.
  useEffect(() => {
    if (!panelContext || panelContext.id === handled.current) return
    handled.current = panelContext.id
    const request = parseOpenContext(panelContext.context)
    if (request) setOpenRequest({ seq: panelContext.id, ...request })
  }, [panelContext])

  return (
    <PageShell className="eval-ui-shell">
      <PageHeader icon={<RefreshCcw size={16} aria-hidden />} title="Eval loop" onClose={onRequestClose} />
      <CycleView
        host={host}
        api={api}
        paneKey={paneId || tabId || 'page'}
        panelSide={panelSide}
        openRequest={openRequest}
        setDirty={setDirty}
      />
    </PageShell>
  )
}
