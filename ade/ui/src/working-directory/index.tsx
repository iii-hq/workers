import {
  Button,
  Card,
  CardBody,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
} from '@iii-dev/console-ui'
import { unwrapEnvelope } from '@iii-dev/console-ui/format'
import { useState } from 'react'

const PROPOSE_ID = 'console::working-directory::propose'

interface WorkingDirectoryProposal {
  sessionId: string
  path: string
  reason?: string
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
    <Card>
      <CardBody className="console-wd-proposal">
        <div className="console-wd-proposal-copy">
          <div className="console-wd-proposal-title">
            Switch working directory?
          </div>
          <div className="console-wd-proposal-path" title={proposal.path}>
            {proposal.path}
          </div>
          {proposal.reason ? (
            <div className="console-wd-proposal-reason">{proposal.reason}</div>
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
      </CardBody>
    </Card>
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
