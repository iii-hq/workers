import { useEffect, useState, type ReactNode } from 'react'
import {
  Badge,
  Skeleton,
  StatusPanel,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
} from '@iii-dev/console-ui'
import { getSurface } from './data'
import { Surface } from './surface'
import { parseReceipt, type SurfaceRecord, type SurfaceReceipt } from './types'

const PAGE_HASH = '#/ext/a2ui'

export function createA2uiTriggerRenderer(host: Host): FunctionTriggerRenderer {
  return {
    id: 'a2ui/page.js#renderer',
    isMatch: (functionId) => functionId.startsWith('a2ui::'),
    tryRender: (message) => renderResult(host, message),
    tryRenderRunning: (message) => renderRunning(message),
    tryRenderPreview: (message) => renderRunning(message),
    primaryTabLabel: 'Interface',
    redactRaw,
  }
}

function renderResult(host: Host, message: FunctionTriggerMessage): ReactNode | null {
  if (message.functionId === 'a2ui::action') return null
  const receipt = parseReceipt(message.output)
  if (!receipt) return null
  if (receipt.status === 'deleted') {
    return (
      <div className="a2ui-trigger">
        <div className="a2ui-trigger-head">
          <Badge>Deleted</Badge>
          <span>{receipt.surface_id}</span>
        </div>
      </div>
    )
  }
  return <GeneratedSurface host={host} receipt={receipt} />
}

function renderRunning(message: FunctionTriggerMessage): ReactNode | null {
  if (
    !['a2ui::generate', 'a2ui::surface::apply', 'a2ui::surface::patch'].includes(
      message.functionId,
    )
  ) {
    return null
  }
  return (
    <div className="a2ui-trigger">
      <div className="a2ui-trigger-head">
        <Badge variant="accent">Composing</Badge>
        <span>Building interface…</span>
      </div>
      <Skeleton className="a2ui-trigger-skeleton" />
    </div>
  )
}

function GeneratedSurface({ host, receipt }: { host: Host; receipt: SurfaceReceipt }) {
  const [surface, setSurface] = useState<SurfaceRecord | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let alive = true
    void getSurface(host, receipt.session_id, receipt.surface_id)
      .then((value) => alive && setSurface(value))
      .catch((cause) => alive && setError(cause instanceof Error ? cause.message : String(cause)))
    return () => {
      alive = false
    }
  }, [host, receipt.session_id, receipt.surface_id, receipt.revision])

  return (
    <div className="a2ui-trigger">
      <div className="a2ui-trigger-head">
        <Badge variant="accent">A2UI</Badge>
        <strong>{receipt.title}</strong>
        <span>{receipt.component_count} components</span>
        <a href={PAGE_HASH}>Open page</a>
      </div>
      {error ? (
        <StatusPanel variant="alert" headline="Could not load generated surface" detail={error} />
      ) : surface ? (
        <Surface host={host} surface={surface} compact />
      ) : (
        <Skeleton className="a2ui-trigger-skeleton" />
      )}
    </div>
  )
}

function redactRaw(value: unknown): unknown {
  return redact(value, new WeakSet<object>())
}

function redact(value: unknown, seen: WeakSet<object>): unknown {
  if (value == null || typeof value !== 'object') return value
  if (seen.has(value)) return '[circular]'
  seen.add(value)
  if (Array.isArray(value)) return value.map((item) => redact(item, seen))
  const output: Record<string, unknown> = {}
  for (const [key, item] of Object.entries(value)) {
    output[key] = ['data', 'data_model', 'context', 'messages'].includes(key)
      ? '[redacted by a2ui]'
      : redact(item, seen)
  }
  return output
}
