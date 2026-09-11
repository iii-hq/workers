import {
  Button,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
} from '@iii-dev/console-ui'
import { useState } from 'react'

const PROPOSE_ID = 'console::working-directory::propose'

interface WorkingDirectoryProposal {
  sessionId: string
  path: string
  reason?: string
}

function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const object = value as Record<string, unknown>
  return Array.isArray(object.content) && 'details' in object
    ? object.details
    : value
}

function parseProposal(value: unknown): WorkingDirectoryProposal | null {
  const output = unwrapEnvelope(value)
  if (!output || typeof output !== 'object' || Array.isArray(output))
    return null
  const proposal = output as Record<string, unknown>
  if (
    typeof proposal.session_id !== 'string' ||
    !proposal.session_id.trim() ||
    typeof proposal.path !== 'string' ||
    !proposal.path.trim() ||
    proposal.requires_confirmation !== true
  ) {
    return null
  }
  return {
    sessionId: proposal.session_id.trim(),
    path: proposal.path.trim(),
    reason:
      typeof proposal.reason === 'string' && proposal.reason.trim()
        ? proposal.reason.trim()
        : undefined,
  }
}

function ProposalCard({
  host,
  proposal,
}: {
  host: Host
  proposal: WorkingDirectoryProposal
}) {
  const [applied, setApplied] = useState(false)
  const canApply = !!host.chat?.requestWorkingDirectoryChange

  const apply = () => {
    const accepted = host.chat?.requestWorkingDirectoryChange?.({
      sessionId: proposal.sessionId,
      path: proposal.path,
    })
    if (accepted) setApplied(true)
  }

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '10px 12px',
        border: '1px solid var(--color-edge)',
        borderRadius: 8,
        background: 'var(--color-panel-raised)',
      }}
    >
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ color: 'var(--color-ink)', fontWeight: 500 }}>
          Switch working directory?
        </div>
        <div
          title={proposal.path}
          style={{
            marginTop: 3,
            overflow: 'hidden',
            color: 'var(--color-ink-faint)',
            fontFamily: 'var(--font-mono)',
            fontSize: 12,
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {proposal.path}
        </div>
        {proposal.reason ? (
          <div
            style={{
              marginTop: 3,
              color: 'var(--color-ink-faint)',
              fontSize: 12,
            }}
          >
            {proposal.reason}
          </div>
        ) : null}
      </div>
      <Button
        size="sm"
        variant={applied ? 'ghost' : 'primary'}
        disabled={applied || !canApply}
        onClick={apply}
      >
        {applied ? 'Using for chat' : 'Use for chat'}
      </Button>
    </div>
  )
}

function render(host: Host, message: FunctionTriggerMessage) {
  if (message.functionId !== PROPOSE_ID || message.running) return null
  const proposal = parseProposal(message.output)
  return proposal ? <ProposalCard host={host} proposal={proposal} /> : null
}

function FunctionIdLabel() {
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>console::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>
        working-directory::propose
      </span>
    </>
  )
}

export function createWorkingDirectoryProposalRenderer(
  host: Host,
): FunctionTriggerRenderer {
  return {
    id: 'console/workspace-proposal.js#working-directory',
    isMatch: (functionId) => functionId === PROPOSE_ID,
    tryRender: (message) => render(host, message),
    FunctionIdLabel,
    metadata: { display: true },
  }
}
