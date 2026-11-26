import type { Host } from '@iii-dev/console-ui'
import { Button, ConfirmDialog } from '@iii-dev/console-ui'
import { useState } from 'react'
import { useLaunch } from './asked'
import { canOpenChat, openChatFor } from './handoff'
import { useActions } from './hub'
import { minutesLabel, type RowTone } from './model'
import { Dot, ErrorPanel, Notice, Spinner, useNow } from './parts'
import type { AgentRun, AgentState } from './types'

const ACTIVE: AgentState[] = ['starting', 'working', 'waiting_for_you', 'waiting', 'stalled']
const TONE: Record<AgentState, RowTone> = {
  starting: 'live',
  working: 'live',
  waiting_for_you: 'warn',
  // A wake it armed: it waits on an event, not on the person.
  waiting: 'edge',
  stalled: 'warn',
  done: 'ok',
  stopped: 'edge',
}

/** The agent's hand-off, alive: state, elapsed, what it is doing now, when it last did something. */
export function AgentRow({
  host,
  agent,
  note,
  branch,
}: {
  host: Host
  agent: AgentRun
  /** Something the person still has to do for the hand-off to work (for example confirm the folder). */
  note?: string
  /** The branch the agent commits on (its copy's), for what stopping keeps. */
  branch?: string | null
}) {
  const now = useNow(1000)
  const { busy, error, call } = useActions(host)
  const [confirm, setConfirm] = useState(false)
  const started = Date.parse(agent.started_at)
  const last = agent.last_activity_at ? Date.parse(agent.last_activity_at) : null
  const idle = last ? now - last : 0
  const canOpen = !!agent.session_id && canOpenChat(host)
  const openChat = canOpen ? () => openChatFor(host, agent.session_id as string) : null
  const active = ACTIVE.includes(agent.state)
  const label: Record<AgentState, string> = {
    starting: 'Starting',
    working: 'Working',
    waiting_for_you: 'Waiting for you',
    waiting: 'Waiting',
    stalled: `No progress for ${minutesLabel(idle)}`,
    done: 'Finished',
    stopped: 'Stopped',
  }
  const ms = (active ? now : (last ?? now)) - started
  const elapsed = minutesLabel(ms)
  // "Starting · 1 min" only once it has really been a minute.
  const showElapsed = !(agent.state === 'starting' && ms < 60_000)
  // Two faint lines under the state: what it said it is doing; when it last did a step, how many.
  const meta = [
    last ? `last step ${idle < 60_000 ? 'just now' : `${minutesLabel(idle)} ago`}` : 'no step yet',
    agent.steps ? `${agent.steps} ${agent.steps === 1 ? 'step' : 'steps'}` : null,
  ]
    .filter(Boolean)
    .join(' · ')
  // Quiet on purpose: the chat is secondary while it works, and stopping is a ghost that asks first.
  const stop = active ? (
    <Button size="sm" variant="ghost" disabled={busy === 'stop'} onClick={() => setConfirm(true)}>
      Stop the agent
    </Button>
  ) : null
  const chatButton = (name: string, primary: boolean) =>
    openChat ? (
      <Button size="sm" variant={primary ? 'primary' : 'secondary'} onClick={openChat}>
        {name}
      </Button>
    ) : null

  const acts = (
    <span className="iii-cloud-agent-acts">
      {agent.state === 'waiting_for_you'
        ? chatButton("Answer in your agent's chat", true)
        : agent.state === 'stalled'
          ? null
          : chatButton("Open your agent's chat", false)}
      {stop}
    </span>
  )
  const faint = (
    <>
      {agent.note ? <span className="iii-cloud-psmall">{agent.note}</span> : null}
      <span className="iii-cloud-psmall">{meta}</span>
      {note ? <span className="iii-cloud-psmall">{note}</span> : null}
    </>
  )

  // A grey panel: its mark, what it does, and the chat and Stop on the right (under it on a phone).
  return (
    <div className="iii-cloud-agent" data-state={agent.state}>
      {agent.state === 'stalled' ? (
        <>
          <Notice
            variant="warn"
            headline={label.stalled}
            detail={`Started ${elapsed} ago. The agent has not reported a step for a while; it may be stuck or waiting on something.`}
            action={chatButton("Open your agent's chat", true)}
          />
          {agent.current ? <span className="iii-cloud-agent-now">now: {agent.current}</span> : null}
          {faint}
          {acts}
        </>
      ) : (
        <>
          <span className="iii-cloud-agent-mark">
            {agent.state === 'working' || agent.state === 'starting' ? <Spinner /> : <Dot tone={TONE[agent.state]} />}
          </span>
          <span className="iii-cloud-agent-text">
            <span className="iii-cloud-agent-line">
              <span className="iii-cloud-agent-head">
                <strong aria-live="polite">{label[agent.state]}</strong>
                {showElapsed ? <span>{active ? ` · ${elapsed}` : ` · took ${elapsed}`}</span> : null}
              </span>
              {agent.state === 'waiting' ? (
                agent.waiting_for ? (
                  <span className="iii-cloud-agent-now"> · for {agent.waiting_for}</span>
                ) : null
              ) : agent.current && active ? (
                <span className="iii-cloud-agent-now"> · now: {agent.current}</span>
              ) : null}
            </span>
            {agent.state === 'waiting_for_you' && agent.question ? (
              <span className="iii-cloud-agent-q">{agent.question}</span>
            ) : null}
            {faint}
          </span>
          {acts}
        </>
      )}
      {error ? <ErrorPanel error={error} /> : null}
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title="Stop your agent?"
        description={
          branch ? `Its commits so far stay on ${branch}.` : 'Its commits so far stay in its copy of your code.'
        }
        confirmLabel="Stop the agent"
        cancelLabel="Cancel"
        tone="danger"
        onConfirm={async () => {
          await call('stop', 'iii-cloud::handoff::stop', { id: agent.handoff_id })
          setConfirm(false)
        }}
      />
    </div>
  )
}

/** How the last hand-off reached a chat: delivered to the agent's chat, or an error that is never silent. */
export function LaunchNote() {
  const launch = useLaunch()
  if (launch?.kind === 'sent') {
    return (
      <div className="iii-cloud-body">
        <span className="iii-cloud-psmall">Sent to your agent's chat</span>
      </div>
    )
  }
  if (launch?.kind === 'waiting') {
    return (
      <div className="iii-cloud-body">
        <span className="iii-cloud-psmall">Sent to your agent. Waiting for it to pick this up.</span>
      </div>
    )
  }
  if (launch?.kind === 'error') {
    return (
      <div className="iii-cloud-body">
        <Notice
          variant="alert"
          headline="Couldn't open a chat for your agent"
          detail="Open a chat yourself, or try again."
          action={
            <Button size="sm" variant="secondary" onClick={launch.retry}>
              Try again
            </Button>
          }
        />
      </div>
    )
  }
  return null
}
