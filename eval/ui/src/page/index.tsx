import { type Host, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { Eye } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createEvalApi } from '../api'
import { MonitorView } from './monitor/MonitorView'
import { type OpenRequest, parseOpenContext } from './monitor/shell-state'

export function EvalPage({
  host,
  panelSide = 'left',
  tabId = '',
  paneId,
  panelContext,
  commands,
  onRequestClose,
  setDirty,
}: { host: Host } & Partial<PageRenderProps>) {
  const api = useMemo(() => createEvalApi(host), [host])
  const paneKey = paneId || tabId || 'page'
  const [openRequest, setOpenRequest] = useState<OpenRequest | null>(null)
  const handledContext = useRef(-1)

  // `host.panels.open` from the palette or a command: an analysis selected, or
  // the Analyze field focused.
  useEffect(() => {
    if (!panelContext || panelContext.id === handledContext.current) return
    handledContext.current = panelContext.id
    const request = parseOpenContext(panelContext.id, panelContext.context)
    if (request) setOpenRequest(request)
  }, [panelContext])

  const openHandled = useCallback(() => setOpenRequest(null), [])

  return (
    <PageShell className="eval-ui-shell">
      <PageHeader icon={<Eye size={16} aria-hidden />} title="Session monitor" onClose={onRequestClose} />
      <MonitorView
        host={host}
        api={api}
        paneKey={paneKey}
        panelSide={panelSide}
        openRequest={openRequest}
        onOpenHandled={openHandled}
        commands={commands}
        setDirty={setDirty}
      />
    </PageShell>
  )
}
