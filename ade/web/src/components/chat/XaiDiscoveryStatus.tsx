import { useEffect, useRef, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { LiveRegion } from '@/components/ui/LiveRegion'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { useLiveAnnouncer } from '@/hooks/use-live-announcer'
import type { DiscoveryOutcome, DiscoveryStatus } from '@/lib/models-catalog'

const COPY: Record<Exclude<DiscoveryOutcome, 'success'>, [string, string]> = {
  billing: [
    'Credits or spending limit',
    'xAI could not list models because the team has run out of credits or reached its monthly spending limit. Add credits or adjust the limit in xAI, then try again.',
  ],
  authentication: [
    'Authentication failed',
    'xAI rejected the credential. Check the API key in settings and try again.',
  ],
  permission: [
    'Access denied',
    'xAI did not allow the model lookup. Check the team and API key permissions.',
  ],
  rate_limit: [
    'Rate limit reached',
    'xAI has rate-limited model lookups. Wait a moment and try again.',
  ],
  unavailable: [
    'xAI unavailable',
    'Models could not be retrieved right now. Try again shortly.',
  ],
  invalid_response: [
    'Invalid response from xAI',
    'The lookup did not return a valid model list. Try again.',
  ],
  empty: [
    'No chat models available',
    'The lookup completed successfully but returned no chat models.',
  ],
  not_configured: [
    'Configure xAI',
    'Add a credential in settings to look up models.',
  ],
}

/** One body shared by the desktop popover and the existing phone sheet. */
export function XaiDiscoveryStatus({
  status,
  onRetry,
  unavailable,
}: {
  status: DiscoveryStatus
  onRetry?: () => Promise<void>
  unavailable?: boolean
}) {
  const [pending, setPending] = useState(false)
  const [retryFailed, setRetryFailed] = useState(false)
  const { announcement, announce } = useLiveAnnouncer()
  const container = useRef<HTMLDivElement>(null)
  const previousOutcome = useRef(status.outcome)
  const retryOrigin = useRef(false)
  useEffect(() => {
    if (
      previousOutcome.current !== status.outcome &&
      (status.outcome === 'success' || status.outcome === 'empty')
    ) {
      announce(
        status.outcome === 'success'
          ? 'xAI models updated.'
          : 'xAI lookup complete: no chat models available.',
      )
      if (retryOrigin.current && status.outcome === 'success') {
        container.current
          ?.closest('section')
          ?.querySelector<HTMLButtonElement>(
            '[data-model-option]:not(:disabled)',
          )
          ?.focus()
      }
    }
    previousOutcome.current = status.outcome
  }, [status.outcome, announce])

  async function retry() {
    if (pending || !onRetry) return
    retryOrigin.current = true
    setPending(true)
    setRetryFailed(false)
    announce('Checking xAI models.')
    try {
      await onRetry()
    } catch {
      setRetryFailed(true)
      announce(
        'The check could not be completed. The last diagnostic has been kept.',
      )
    } finally {
      setPending(false)
    }
  }

  const copy = status.outcome === 'success' ? null : COPY[status.outcome]
  return (
    <div ref={container}>
      <LiveRegion announcement={announcement} />
      {copy ? (
        <StatusPanel
          variant={
            status.outcome === 'empty' || status.outcome === 'not_configured'
              ? 'info'
              : 'warn'
          }
          headline={copy[0]}
          detail={
            <div className="flex min-w-0 flex-col gap-3">
              <p>{copy[1]}</p>
              {status.stale ? (
                <Badge variant="warn">
                  Previous catalog — may be out of date
                </Badge>
              ) : null}
              {retryFailed ? (
                <p>
                  The check could not be completed. The last diagnostic has been kept.
                </p>
              ) : null}
              <div className="flex min-w-0 flex-col items-stretch gap-2">
                {onRetry ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="lg"
                    disabled={pending || unavailable}
                    aria-busy={pending}
                    onClick={() => void retry()}
                  >
                    {pending ? 'Checking…' : 'Check again'}
                  </Button>
                ) : null}
                <Button asChild variant="ghost" size="lg">
                  <a
                    href="https://console.x.ai"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    Open xAI console
                  </a>
                </Button>
              </div>
              <details>
                <summary className="cursor-pointer rounded-sm py-3 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus">
                  Technical details
                </summary>
                <p>
                  Source: xAI · Operation: model lookup
                  {status.http_status ? ` · HTTP ${status.http_status}` : ''}
                  {status.code ? ` / ${status.code}` : ''}
                </p>
              </details>
            </div>
          }
        />
      ) : null}
    </div>
  )
}
