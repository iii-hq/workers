import type { Host } from '@iii-dev/console-ui'
import { Button, ConfirmDialog } from '@iii-dev/console-ui'
import { useState } from 'react'
import { useLaunch } from './asked'
import { canOpenChat, openChatFor } from './handoff'
import { hubOf, useActions } from './hub'
import { ErrorPanel, Notice } from './parts'
import type { AgentRun, AgentState } from './types'

const ACTIVE: AgentState[] = ['starting', 'working', 'waiting_for_you', 'waiting', 'stalled']

/** Stop, which asks first: the agent's commits so far stay where they are. */
function StopButton({
  host,
  agent,
  branch,
  label,
  variant,
}: {
  host: Host
  agent: AgentRun
  branch?: string | null
  label: string
  variant: 'ghost' | 'secondary'
}) {
  const { busy, error, call } = useActions(host)
  const [confirm, setConfirm] = useState(false)
  return (
    <>
      <Button size="sm" variant={variant} disabled={busy === 'stop'} onClick={() => setConfirm(true)}>
        {label}
      </Button>
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
          const stopped = await call('stop', 'iii-cloud::handoff::stop', { id: agent.handoff_id })
          // The person stopped it: the screen does not start another by itself when the check ends.
          if (stopped !== null) hubOf(host).markStopped(hubOf(host).get().status?.project?.environment?.id ?? '', true)
          setConfirm(false)
        }}
      />
    </>
  )
}

/** "Open chat" and a visible "Stop": the end of the Your agent card's header. */
export function AgentControls({
  host,
  agent,
  branch,
  chat = true,
}: {
  host: Host
  agent: AgentRun
  branch?: string | null
  /** The "Open chat" link (false: the screen already has a button for it). */
  chat?: boolean
}) {
  const open =
    chat && agent.session_id && canOpenChat(host) ? () => openChatFor(host, agent.session_id as string) : null
  return (
    <span className="iii-cloud-agent-acts">
      {open ? (
        <button type="button" className="iii-cloud-linkbtn iii-cloud-small" onClick={open}>
          Open chat
        </button>
      ) : null}
      {ACTIVE.includes(agent.state) ? (
        <StopButton host={host} agent={agent} branch={branch} label="Stop" variant="secondary" />
      ) : null}
    </span>
  )
}

/** How the last hand-off reached a chat: delivered to the agent's chat, or an error that is never silent. */
export function LaunchNote({ check }: { check?: string | null }) {
  const raw = useLaunch()
  // What it says is about the check it was pressed for: a newer check (or another environment's) shows nothing.
  const launch =
    raw &&
    (raw.kind === 'sent' || raw.kind === 'told' || raw.kind === 'asks' || raw.kind === 'waiting') &&
    raw.check !== (check ?? undefined)
      ? null
      : raw
  if (launch?.kind === 'sent') {
    return (
      <div className="iii-cloud-body">
        <span className="iii-cloud-psmall">Sent to your agent's chat</span>
      </div>
    )
  }
  if (launch?.kind === 'asks') {
    return (
      <div className="iii-cloud-body">
        <span className="iii-cloud-psmall">Answer your agent's question in the chat first.</span>
      </div>
    )
  }
  if (launch?.kind === 'told') {
    return (
      <div className="iii-cloud-body">
        <span className="iii-cloud-psmall">Tell it in the chat.</span>
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
