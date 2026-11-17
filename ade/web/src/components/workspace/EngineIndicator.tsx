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
import { Button } from '@/components/ui/Button'
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
 * Which iii engine this ADE is connected to, in view at all times: a
 * status dot and the engine's `host:port` in the header. The popover
 * spells it out — the engine URL, the namespace, the WebSocket this page
 * opened, the connection state — with a copy for the address, so a person
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
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let off: (() => void) | undefined
    void getIiiClient()
      .then((client) => {
        if (cancelled) return
        setInfo(client.connection)
        off = client.addConnectionStateListener(setState)
      })
      .catch(() => {
        if (!cancelled) setState('failed')
      })
    return () => {
      cancelled = true
      off?.()
    }
  }, [enabled])

  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1500)
    return () => window.clearTimeout(timer)
  }, [copied])

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
        className={cn(
          'icon-host inline-flex h-8 max-w-56 shrink-0 items-center gap-2 rounded-field bg-card px-3 font-inter text-body font-medium text-muted-foreground transition-[background-color,color] duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] hover:bg-surface-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none data-open:bg-surface-hover data-open:text-foreground',
          className,
        )}
      >
        {dot}
        <span className="min-w-0 truncate">{label}</span>
      </PopoverTrigger>
      <PopoverContent align="end" sideOffset={8} className="w-80 gap-3 p-3">
        <PopoverHeader>
          <PopoverTitle className="flex items-center gap-2">
            {dot}
            Engine
          </PopoverTitle>
          <PopoverDescription>{status}</PopoverDescription>
        </PopoverHeader>
        <dl className="flex flex-col gap-2.5 font-inter">
          <div className="flex flex-col gap-0.5">
            <dt className="text-xs text-muted-foreground">Engine</dt>
            <dd className="min-w-0 break-all text-sm leading-5 text-foreground">
              {info?.engineUrl ??
                (info ? (
                  <span className="text-muted-foreground">
                    Not reported by this ADE worker
                  </span>
                ) : (
                  '…'
                ))}
            </dd>
          </div>
          <div className="flex flex-col gap-0.5">
            <dt className="text-xs text-muted-foreground">Namespace</dt>
            <dd className="min-w-0 break-all text-sm leading-5 text-foreground">
              {info ? namespace : '…'}
            </dd>
          </div>
          <div className="flex flex-col gap-0.5">
            <dt className="text-xs text-muted-foreground">
              This page connects through
            </dt>
            <dd className="min-w-0 break-all text-sm leading-5 text-foreground">
              {info?.wsUrl ?? '…'}
            </dd>
          </div>
        </dl>
        {address ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="self-start"
            onClick={() => {
              void copyTextToClipboard(address).then(() => setCopied(true))
            }}
          >
            {copied ? (
              <Check className="size-4" aria-hidden />
            ) : (
              <Copy className="size-4" aria-hidden />
            )}
            {copied ? 'Copied' : 'Copy engine address'}
          </Button>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}
