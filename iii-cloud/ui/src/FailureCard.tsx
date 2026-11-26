import type { Host } from '@iii-dev/console-ui'
import { Button, CollapsibleCard, CollapsibleCardContent, CollapsibleCardTrigger } from '@iii-dev/console-ui'
import { ExternalLink } from 'lucide-react'
import { type ReactNode, useEffect, useRef } from 'react'
import { useLaunch } from './asked'
import { handOff, openAgentChat } from './handoff'
import { activeSameKind, cloudStageLabel, failedStage, failureWords, sendLabel } from './model'
import type { Deploy, Status } from './types'

/** The failure in one plain sentence: the worker's title, else the stage and what happened there. */
export function failureTitle(d: Deploy): string {
  if (d.failure?.title) return d.failure.title
  return `The deploy failed at ${failedStage(d)}${d.error ? `: ${failureWords(d.error)}` : ''}`
}

/** The failing stage's last lines, as the worker kept them, and what to call them. */
export function failureTail(d: Deploy): { label: string; lines: string[] } {
  const f = d.failure
  return f
    ? { label: `Last lines of ${cloudStageLabel(f.stage)}`, lines: f.log_tail }
    : { label: 'Last log lines', lines: d.log_tail }
}

/** Log lines in the shared pre, scrolled to the end, where the error is. */
export function LogLines({ lines }: { lines: string[] }) {
  const ref = useRef<HTMLPreElement>(null)
  const text = lines.join('\n')
  useEffect(() => {
    const el = ref.current
    if (el) el.scrollTop = el.scrollHeight
  }, [text])
  return (
    <pre className="iii-cloud-log" ref={ref}>
      {text}
    </pre>
  )
}

/**
 * A failed deploy in words (Mng2, Mng10): the plain title, what happened, the fix, and whether anyone saw it.
 * `log` shows the failing stage's last lines (folded open on a phone); the Deploys tab turns it off on a
 * wide pane, where its log panel has them. `meta` goes right under the title.
 */
export function FailureCard({
  host,
  status,
  deploy,
  serving = false,
  narrow = false,
  log = true,
  meta,
}: {
  host: Host
  status: Status
  deploy: Deploy
  /** An earlier release keeps serving. */
  serving?: boolean
  narrow?: boolean
  log?: boolean
  meta?: ReactNode
}) {
  const f = deploy.failure
  const launching = useLaunch()?.kind === 'launching'
  const working = activeSameKind(status, 'deploy_fix')
  const hint = f ? f.hint : deploy.error?.hint
  const env = status.project?.environment?.name ?? 'the environment'
  const tail = failureTail(deploy)
  const size = narrow ? 'lg' : 'md'
  const lines = tail.lines.length ? <LogLines lines={tail.lines} /> : null
  return (
    <div className="iii-cloud-failure">
      <h2 className="iii-cloud-h">{failureTitle(deploy)}</h2>
      {meta}
      {f?.silent ? <p className="iii-cloud-desc">The command exited without printing an error.</p> : null}
      {hint ? <p className="iii-cloud-desc">{hint}</p> : null}
      {f?.release_command ? (
        <p className="iii-cloud-desc">
          Release command: <span className="iii-cloud-mono">{f.release_command}</span>
        </p>
      ) : null}
      <div className="iii-cloud-row">
        {working ? (
          <Button variant="secondary" size={size} onClick={() => openAgentChat(host, status)}>
            Open your agent's chat
          </Button>
        ) : (
          <Button
            variant="primary"
            size={size}
            disabled={launching || !host.chat?.compose}
            onClick={() => handOff(host, status, 'deploy_fix', deploy)}
          >
            {sendLabel(status, 'deploy_fix', 'Fix with your agent')}
          </Button>
        )}
        <Button variant="secondary" size={size} asChild>
          <a href={deploy.console_url} target="_blank" rel="noreferrer">
            Open full logs
            <ExternalLink />
          </a>
        </Button>
        <span className="iii-cloud-desc">
          {narrow
            ? 'Your agent works on your computer; you can follow it from here.'
            : 'Your agent gets the log lines below; its fix goes through Check before deploy.'}
        </span>
      </div>
      <p className="iii-cloud-text">
        {serving ? 'The previous release keeps serving.' : `Nothing was live on ${env} then, so nobody saw an error.`}
      </p>
      {log && lines ? (
        narrow ? (
          <CollapsibleCard defaultOpen>
            <CollapsibleCardTrigger className="iii-cloud-fold">Last lines of the log</CollapsibleCardTrigger>
            <CollapsibleCardContent className="iii-cloud-fold-body">{lines}</CollapsibleCardContent>
          </CollapsibleCard>
        ) : (
          <div className="iii-cloud-logbox">
            <span className="iii-cloud-psmall">{tail.label}</span>
            {lines}
          </div>
        )
      ) : null}
    </div>
  )
}
