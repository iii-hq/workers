import { Radio } from 'lucide-react'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import { ObserverRow } from './ObserversListView'
import {
  type ObserverInfo,
  observerIdRequestSchema,
  safeParseRequest,
} from './parsers'

interface ObserverInfoViewProps {
  input: unknown
  /** Parsed observer — the caller (index.tsx) falls back to panes on null. */
  observer?: ObserverInfo
  running?: boolean
}

/**
 * `engine::observers::get` — one observer: the observed function with its
 * args and filters, its target, state, policy and counters. Mirrors
 * `FunctionInfoView`'s layout so the inspector cards read as the same
 * instrument.
 */
export function ObserverInfoView({
  input,
  observer,
  running,
}: ObserverInfoViewProps) {
  const req = safeParseRequest(observerIdRequestSchema, input)

  if (running || !observer) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? (
            <Chip>
              <span className="iii-ui-eyebrow">observer</span>
              <span className="ml-1 text-ink break-all">{req.id}</span>
            </Chip>
          ) : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · inspecting observer…
        </div>
      </div>
    )
  }

  const policy = {
    ...(observer.on ? { on: observer.on } : {}),
    ...(observer.deliver ? { deliver: observer.deliver } : {}),
    ...(observer.capture ? { capture: observer.capture } : {}),
    ...(observer.max_fires !== undefined
      ? { max_fires: observer.max_fires }
      : {}),
    ...(observer.expires_in_ms !== undefined
      ? { expires_in_ms: observer.expires_in_ms }
      : {}),
    ...(observer.max_in_flight !== undefined
      ? { max_in_flight: observer.max_in_flight }
      : {}),
    ...(observer.dropped && Object.keys(observer.dropped).length > 0
      ? { dropped: observer.dropped }
      : {}),
    ...(observer.activation_failures &&
    Object.keys(observer.activation_failures).length > 0
      ? { activation_failures: observer.activation_failures }
      : {}),
  }

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={observer.state}
          variant={observer.state === 'active' ? 'accent' : 'warn'}
        />
        {observer.namespace ? (
          <Chip>
            <span className="iii-ui-eyebrow">namespace</span>
            <span className="ml-1 text-ink">{observer.namespace}</span>
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine icon={<Radio />} tone="accent">
        <span className="font-mono text-[13px] text-accent break-all">
          {observer.function_id}
        </span>
      </ActionLine>
      <ul>
        <ObserverRow observer={observer} />
      </ul>
      {Object.keys(policy).length > 0 ? (
        <div className="border-t border-rule-2">
          <div className="border-b border-rule-2 bg-paper-2 px-3 py-1.5 iii-ui-eyebrow">
            delivery and policy
          </div>
          <JsonHighlight code={JSON.stringify(policy, null, 2)} wrap />
        </div>
      ) : null}
    </div>
  )
}
