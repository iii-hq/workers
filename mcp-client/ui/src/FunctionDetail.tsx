import {
  Button,
  type Host,
  type PageRenderProps,
  Skeleton,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { Api } from './api'
import { type FunctionRow, functionName, metaLine } from './presentation'
import { SchemaTable } from './SchemaTable'
import type { FunctionInfo, Server } from './types'

const SKELETON_LINES = [0, 1, 2]

/** The engine's answer for one function id: its contract, or why it could not be read. */
interface Contract {
  functionId: string
  info: FunctionInfo | null
  error: string | null
}

/** Clamped to six lines by CSS; "Show more" only when the text runs past them. */
function Description({ text }: { text: string }) {
  const ref = useRef<HTMLParagraphElement>(null)
  const [open, setOpen] = useState(false)
  const [overflows, setOverflows] = useState(false)
  // Measured while clamped, and again whenever the column's width changes.
  useLayoutEffect(() => {
    const el = ref.current
    if (!el || open) return
    const measure = () => setOverflows(el.scrollHeight > el.clientHeight)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [open, text])
  return (
    <>
      <p ref={ref} className="mcp-ui-detail-desc" data-open={open}>
        {text}
      </p>
      {overflows ? (
        <Button variant="ghost" size="sm" className="mcp-ui-more" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? 'Show less' : 'Show more'}
        </Button>
      ) : null}
    </>
  )
}

/** Column 3 for a tool, a resource function or a prompt: its head, then Test and Input schema. */
export function FunctionDetail({
  api,
  server,
  row,
}: {
  api: Api
  server: Server
  row: FunctionRow
  host: Host
  commands: PageRenderProps['commands']
}) {
  const { functionId } = row
  const [contract, setContract] = useState<Contract | null>(null)
  const [attempt, setAttempt] = useState(0)
  const seq = useRef(0)

  // `server` is a new object on every live read of the page: the list carries no schema, so a tool the
  // server changed is read again then. A reply to an earlier request is dropped.
  useEffect(() => {
    const mine = ++seq.current
    api.functionInfo(functionId).then(
      (info) => {
        if (mine === seq.current) setContract({ functionId, info, error: null })
      },
      (err) => {
        if (mine === seq.current) setContract({ functionId, info: null, error: errorMessage(err) })
      },
    )
  }, [api, functionId, server, attempt])

  // What is held for another function never shows, not even for the first frame after a switch.
  const held = contract?.functionId === functionId ? contract : null
  const schema = held?.info ? (held.info.request_schema ?? null) : undefined

  return (
    <div className="mcp-ui-detail">
      <div className="mcp-ui-detail-head">
        <h2 className="mcp-ui-detail-name">{functionName(server.name, functionId)}</h2>
        {row.description ? <Description key={functionId} text={row.description} /> : null}
        <code className="mcp-ui-detail-meta">{metaLine(functionId, schema, row.group)}</code>
      </div>
      {held?.error ? (
        <StatusPanel
          variant="alert"
          headline="Could not read this function's contract"
          detail={held.error}
          action={
            <Button variant="secondary" size="sm" onClick={() => setAttempt((n) => n + 1)}>
              Retry
            </Button>
          }
        />
      ) : (
        <Tabs defaultValue="test" className="mcp-ui-tabs">
          <TabsList variant="line">
            <TabsTrigger value="test" icon={false}>
              Test
            </TabsTrigger>
            <TabsTrigger value="schema" icon={false}>
              Input schema
            </TabsTrigger>
          </TabsList>
          <TabsContent value="test" />
          <TabsContent value="schema">
            {held?.info ? (
              <SchemaTable schema={schema} label="Input" empty="This function declares no input schema." />
            ) : (
              SKELETON_LINES.map((n) => <Skeleton key={n} className="mcp-ui-skeleton-line" />)
            )}
          </TabsContent>
        </Tabs>
      )}
    </div>
  )
}
