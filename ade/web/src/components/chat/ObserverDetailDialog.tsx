import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Fact, FactList } from '@/components/ui/FactCard'
import { Hint } from '@/components/ui/Hint'
import { ObserverIcon } from '@/components/ui/ObserverIcon'
import type { SessionObserverInfo } from '@/lib/backend/observers'
import { copyTextToClipboard } from '@/lib/clipboard'
import { JsonHighlight } from '@/lib/syntax'

/**
 * What a fire delivers, from the row's data alone: a wake into this chat, or
 * a plain function call. No observed function or target is special-cased —
 * a function that does not exist yet renders exactly like the known ones.
 */
export function deliveryLabel(observer: SessionObserverInfo): string {
  return observer.delivery.kind === 'call'
    ? `calls ${observer.delivery.functionId}`
    : 'notifies this chat'
}

/**
 * The row's lifecycle text, from structured lifecycle data alone. Historical
 * `retired` records do not say how the observer ended, so keep their copy
 * deliberately neutral instead of presenting every retirement as a removal.
 */
export function lifecycleNote(observer: SessionObserverInfo): string | null {
  if (observer.fired) {
    switch (observer.retirementReason) {
      case 'once_consumed':
        return 'once · consumed automatically'
      case 'max_fires':
        return 'fire limit reached'
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'removed'
      case 'invalidated':
        return 'invalidated'
      case 'exhausted':
        return 'exhausted'
    }

    // Records normally repeat these lifecycle events as both outcome and
    // retirement reason. Tolerate a partially enriched record while
    // preserving the same distinct labels.
    switch (observer.outcome) {
      case 'expired':
        return 'expired'
      case 'unregistered':
        return 'removed'
      case 'invalidated':
        return 'invalidated'
      default:
        return 'retired'
    }
  }
  const parts: string[] = []
  if (observer.once) parts.push('once')
  if ((observer.fires ?? 0) > 0)
    parts.push(`${observer.fires} fire${observer.fires === 1 ? '' : 's'}`)
  if (observer.maxFires !== undefined) parts.push(`max ${observer.maxFires}`)
  if (observer.expiresAt !== undefined)
    parts.push(`until ${new Date(observer.expiresAt).toLocaleString()}`)
  return parts.length > 0 ? parts.join(' · ') : null
}

function isEmptyObject(value: unknown): boolean {
  if (value === null || value === undefined) return true
  if (typeof value === 'object')
    return Object.keys(value as Record<string, unknown>).length === 0
  return false
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

/** FCM-style labeled JSON section: tracked header row + wrapped highlight. */
function JsonSection({ label, value }: { label: string; value: unknown }) {
  return (
    <div className="border border-rule-2">
      <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 iii-ui-eyebrow">
        {label}
      </div>
      <JsonHighlight code={formatJson(value)} wrap />
    </div>
  )
}

/** Monospace value with a copy affordance for long opaque ids. */
function CopyableId({ value }: { value: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <span className="inline-flex min-w-0 items-baseline gap-1.5">
      <span className="break-all">{value}</span>
      <Hint label={copied ? 'Copied' : 'Copy'}>
        <button
          type="button"
          onClick={() => {
            // Helper, not navigator.clipboard: the API is undefined over
            // `http://<LAN-IP>` (insecure context) and the raw call no-ops.
            void copyTextToClipboard(value).then((ok) => {
              if (!ok) return
              setCopied(true)
              window.setTimeout(() => setCopied(false), 1200)
            })
          }}
          className="shrink-0 self-center text-ink-ghost hover:text-ink transition-colors"
          aria-label={copied ? 'copied' : 'copy id'}
        >
          {copied ? (
            <Check size={16} aria-hidden />
          ) : (
            <Copy size={16} aria-hidden />
          )}
        </button>
      </Hint>
    </span>
  )
}

/**
 * One observer's detail: the function it observes with its args and
 * filters, what it delivers, its lifetime and id, with the remove (or, for
 * an inactive row, dismiss) button. Opened from the chat's side panel.
 */
export function ObserverDetailDialog({
  observer,
  busy,
  onOpenChange,
  onDispose,
  onDismiss,
}: {
  observer: SessionObserverInfo | null
  busy: boolean
  onOpenChange: (open: boolean) => void
  onDispose: (id: string) => void
  /** An inactive row's dismiss button (it has nothing left to remove). */
  onDismiss?: (id: string) => void
}) {
  return (
    <Dialog open={observer !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogTitle className="text-[14px]">
          <span
            className="mr-2 inline-flex align-baseline text-ink-ghost"
            aria-hidden
          >
            <ObserverIcon size={16} className="fill-ink-ghost" />
          </span>
          {observer ? observer.label || observer.functionId : ''}
        </DialogTitle>
        <DialogDescription className="mt-1">
          observer created by this conversation's agent.
        </DialogDescription>
        {observer ? (
          <div className="mt-4 space-y-4">
            <FactList className="gap-y-1.5">
              <Fact label="Observes" mono>
                {observer.functionId}
              </Fact>
              {observer.namespace ? (
                <Fact label="Namespace" mono muted>
                  {observer.namespace}
                </Fact>
              ) : null}
              <Fact label="Delivers" mono>
                {deliveryLabel(observer)}
              </Fact>
              <Fact label="Lifetime" mono>
                {observer.fired
                  ? (lifecycleNote(observer) ?? 'retired')
                  : observer.once
                    ? 'once — retires after first fire'
                    : (lifecycleNote(observer) ?? 'until removed')}
              </Fact>
              {observer.createdAt !== undefined ? (
                <Fact label="Created" mono muted>
                  {new Date(observer.createdAt).toLocaleString()}
                </Fact>
              ) : null}
              <Fact label="Observer" mono muted>
                <CopyableId value={observer.id} />
              </Fact>
            </FactList>
            {isEmptyObject(observer.args) ? null : (
              <JsonSection label="args" value={observer.args} />
            )}
            {isEmptyObject(observer.filters) ? null : (
              <JsonSection label="filters" value={observer.filters} />
            )}
            {observer.conditions && observer.conditions.length > 0 ? (
              <JsonSection label="conditions" value={observer.conditions} />
            ) : null}
            <div className="flex justify-end">
              {/* An inactive row has no live observer left to remove —
                  offering it would only produce a guaranteed error. */}
              {observer.fired ? (
                <Button
                  type="button"
                  variant="pill"
                  size="sm"
                  onClick={() => onDismiss?.(observer.id)}
                >
                  dismiss
                </Button>
              ) : (
                <Button
                  type="button"
                  variant="pill"
                  size="sm"
                  disabled={busy}
                  onClick={() => onDispose(observer.id)}
                >
                  {busy ? 'removing…' : 'remove'}
                </Button>
              )}
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  )
}
