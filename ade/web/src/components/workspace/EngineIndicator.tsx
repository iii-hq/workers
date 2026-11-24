import { Check, Copy, Server } from 'lucide-react'
import { useEffect, useState } from 'react'
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import { IconButton } from '@/components/ui/IconButton'
import { ToolbarPill } from '@/components/ui/ToolbarPill'
import { getDefaultBackend } from '@/lib/backend'
import { copyTextToClipboard } from '@/lib/clipboard'
import {
  type EngineConnectionInfo,
  getIiiClient,
  type IIIConnectionState,
} from '@/lib/iii-client'
import { cn } from '@/lib/utils'

/**
 * The engine as the header names it: `host:port` of the engine the ADE
 * worker forwards to, or of the page's own WebSocket when the worker did
 * not say. The scheme and path add nothing a reader uses to tell two
 * engines apart; a default port is the host alone.
 */
export function engineHostLabel(url: string): string {
  try {
    const parsed = new URL(url)
    return parsed.port ? `${parsed.hostname}:${parsed.port}` : parsed.hostname
  } catch {
    return url
  }
}

/** The engine the indicator names: the reported one first, then the proxy. */
export function engineAddress(info: EngineConnectionInfo): string {
  return info.engineUrl ?? info.wsUrl
}

export function connectionStatusLabel(
  state: IIIConnectionState | 'failed' | 'unknown',
): string {
  switch (state) {
    case 'connected':
      return 'Connected'
    case 'connecting':
      return 'Connecting…'
    case 'reconnecting':
      return 'Reconnecting…'
    case 'failed':
      return 'Unable to connect'
    default:
      return 'Checking…'
  }
}

function statusTone(state: IIIConnectionState | 'failed' | 'unknown'): string {
  if (state === 'connected') return 'bg-ok'
  if (state === 'failed') return 'bg-destructive'
  return 'bg-warn'
}

interface EngineIndicatorProps {
  /** Real backend only: the demo has no engine to name. */
  enabled?: boolean
  /** `pill` is the header control with its popover; `row` a one-line
 readout for the phone's workspace sheet. */
  variant?: 'pill' | 'row'
  className?: string
}

/**
 * One fact in the engine popover: its name, the value, and a copy button
 * beside it (every value here is something a person pastes elsewhere). A
 * missing value shows its placeholder and has nothing to copy.
 */
function EngineFact({
  label,
  copyName,
  value,
  placeholder,
}: {
  label: string
  /** What the copy button copies, for its accessible name. */
  copyName: string
  value: string | null
  placeholder: string
}) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1500)
    return () => window.clearTimeout(timer)
  }, [copied])
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="flex min-w-0 items-start gap-2">
        <span
          className={cn(
            'min-w-0 flex-1 break-all text-sm leading-5',
            value ? 'text-foreground' : 'text-muted-foreground',
          )}
        >
          {value ?? placeholder}
        </span>
        {value ? (
          <IconButton
            label={copied ? `Copied the ${copyName}` : `Copy the ${copyName}`}
            tooltip={copied ? 'Copied' : 'Copy'}
            className="-my-1.5 shrink-0"
            onClick={() => {
              void copyTextToClipboard(value).then((ok) => {
                if (ok) setCopied(true)
              })
            }}
          >
            {copied ? (
              <Check className="size-4" aria-hidden />
            ) : (
              <Copy className="size-4" aria-hidden />
            )}
          </IconButton>
        ) : null}
      </dd>
    </div>
  )
}

/**
 * Which iii engine this ADE is connected to, in view at all times: a
 * status dot and the engine's `host:port` in the header. The popover
 * spells it out — the engine URL, the namespace, the WebSocket this page
 * opened, the connection state — each with a copy button, so a person
 * running several engines can tell at a glance which one a chat goes to.
 */
export function EngineIndicator({
  enabled = getDefaultBackend().id === 'real',
  variant = 'pill',
  className,
}: EngineIndicatorProps) {
  const [info, setInfo] = useState<EngineConnectionInfo | null>(null)
  const [state, setState] = useState<IIIConnectionState | 'failed' | 'unknown'>(
    'unknown',
  )

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let off: (() => void) | undefined
    let retry: number | undefined
    // A failed bootstrap is not cached (`getIiiClient` starts afresh, or
    // joins whichever attempt the rest of the app has in flight), so the
    // indicator keeps asking, backing off, until a client comes up.
    const connect = (attempt: number) => {
      void getIiiClient()
        .then((client) => {
          if (cancelled) return
          setInfo(client.connection)
          off = client.addConnectionStateListener(setState)
        })
        .catch(() => {
          if (cancelled) return
          setState('failed')
          retry = window.setTimeout(
            () => connect(attempt + 1),
            Math.min(1000 * 2 ** attempt, 30_000),
          )
        })
    }
    connect(0)
    return () => {
      cancelled = true
      window.clearTimeout(retry)
      off?.()
    }
  }, [enabled])

  if (!enabled) return null

  const address = info ? engineAddress(info) : null
  const label = address ? engineHostLabel(address) : 'engine'
  const status = connectionStatusLabel(state)
  const namespace = info?.namespace ?? 'default'
  const dot = (
    <span
      aria-hidden
      className={cn('size-2 shrink-0 rounded-full', statusTone(state))}
    />
  )

  if (variant === 'row') {
    return (
      <div
        role="status"
        className={cn(
          'flex min-h-12 items-center gap-3 rounded-sm bg-surface px-3 font-sans text-base text-ink-faint',
          className,
        )}
        aria-label={`Engine: ${label}, ${status}`}
      >
        <Server className="size-5 shrink-0" aria-hidden />
        <span className="min-w-0 flex-1 truncate">
          <span className="text-sm font-medium text-ink">{label}</span>
          <span className="ml-2 text-sm">{status}</span>
        </span>
        {dot}
      </div>
    )
  }

  return (
    <Popover>
      <PopoverTrigger
        aria-label={`Engine: ${label}, ${status}`}
        title={`Connected to ${label}`}
        render={<ToolbarPill className={cn('max-w-56', className)} />}
      >
        {dot}
        <span className="min-w-0 truncate">{label}</span>
      </PopoverTrigger>
      <PopoverContent align="end" sideOffset={8} className="w-80">
        <PopoverHeader>
          <div className="flex items-center gap-2">
            {dot}
            <PopoverTitle>Engine</PopoverTitle>
          </div>
          <PopoverDescription>{status}</PopoverDescription>
        </PopoverHeader>
        <dl className="flex flex-col gap-2.5 font-inter">
          <EngineFact
            label="Engine"
            copyName="engine URL"
            value={info?.engineUrl ?? null}
            placeholder={info ? 'Not reported by this ADE worker' : '…'}
          />
          <EngineFact
            label="Namespace"
            copyName="namespace"
            value={info ? namespace : null}
            placeholder="…"
          />
          <EngineFact
            label="This page connects through"
            copyName="page's WebSocket URL"
            value={info?.wsUrl ?? null}
            placeholder="…"
          />
        </dl>
      </PopoverContent>
    </Popover>
  )
}
