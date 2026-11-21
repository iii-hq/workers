import {
  Button,
  type Host,
  type PageRenderProps,
  ShadcnTabsContent,
  Skeleton,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { serverAccess } from './access'
import type { Api } from './api'
import { type FunctionRow, functionName, metaLine } from './presentation'
import { ResourcePicker } from './ResourcePicker'
import { SchemaTable } from './SchemaTable'
import { TestPanel } from './TestPanel'
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
  commands,
}: {
  api: Api
  server: Server
  row: FunctionRow
  host: Host
  commands: PageRenderProps['commands']
}) {
  const { functionId } = row
  const access = serverAccess(server).find((fn) => fn.functionId === functionId)
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
        // A failed re-read (a reconnect) keeps the contract already held: the Test panel and its run stay.
        if (mine !== seq.current) return
        setContract((prev) =>
          prev?.functionId === functionId && prev.info ? prev : { functionId, info: null, error: errorMessage(err) },
        )
      },
    )
  }, [api, functionId, server, attempt])

  // What is held for another function never shows, not even for the first frame after a switch.
  const held = contract?.functionId === functionId ? contract : null
  const schema = held?.info ? (held.info.request_schema ?? null) : undefined
  const skeleton = SKELETON_LINES.map((n) => <Skeleton key={n} className="mcp-ui-skeleton-line" />)
  // `resources::read` and a prompt swap the Test words; the read also its Input half (the uri picker).
  const reads = access?.kind === 'resource' && access.name === 'resources::read'

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
          {/* Kept mounted (hidden) under Input schema, so a run, its result and the payload survive a look at
              the schema. The house TabsContent is this same part; only its typing lacks `keepMounted`. */}
          <ShadcnTabsContent value="test" keepMounted>
            {held?.info && access ? (
              <TestPanel
                key={functionId}
                api={api}
                server={server}
                functionId={functionId}
                name={access.name}
                schema={held.info.request_schema}
                access={access}
                commands={commands}
                runLabel={reads ? 'Read resource' : access.kind === 'prompt' ? 'Get prompt' : undefined}
                input={
                  reads
                    ? (payload, setPayload) => (
                        <ResourcePicker
                          api={api}
                          server={server}
                          uri={typeof payload.uri === 'string' ? payload.uri : ''}
                          onUri={(uri) => setPayload(uri ? { uri } : {})}
                        />
                      )
                    : undefined
                }
              />
            ) : (
              skeleton
            )}
          </ShadcnTabsContent>
          <TabsContent value="schema">
            {held?.info ? (
              <SchemaTable schema={schema} label="Input" empty="This function declares no input schema." />
            ) : (
              skeleton
            )}
          </TabsContent>
        </Tabs>
      )}
    </div>
  )
}
