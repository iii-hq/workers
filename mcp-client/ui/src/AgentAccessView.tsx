import {
  Button,
  type ExtensionIii,
  type LiveAnnouncement,
  LiveRegion,
  StatusDot,
  StatusPanel,
  useConfirm,
  WorkerConfigurationDialog,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Fragment, useState } from 'react'
import { readIds, serverAccess } from './access'
import { type GateRead, writeBlock } from './gate-io'
import { gateStatus, type Verdict } from './gate-rules'
import {
  type AccessItem,
  accessAction,
  accessConfirm,
  accessLists,
  entryRefused,
  readOnlyEndpoint,
  reviewAccessPanel,
} from './presentation'
import type { Server } from './types'

/** The spec's copy marks ids with backticks: they render as code, never as literal backticks. */
export const coded = (text: string) =>
  text.split('`').map((part, i) =>
    i % 2 ? (
      <code key={i} className="mcp-ui-inline-code">
        {part}
      </code>
    ) : (
      part
    ),
  )

type Write = 'allow' | 'update' | 'remove'
const LABEL: Record<Write, string> = { allow: 'Allow reads', update: 'Update rules', remove: 'Remove rules' }

/** Function ids in mono, each with why it counts as what it is. Spans, because a confirm's description is a `<p>`,
 * which cannot hold a `<ul>`. */
function Ids({ items, label }: { items: AccessItem[]; label?: string }) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: a <ul> cannot sit in the confirm's <p>
    <span className="mcp-ui-ids" role="list" aria-label={label}>
      {items.map((item) => (
        // biome-ignore lint/a11y/useSemanticElements: a <li> needs the <ul> above
        <span key={item.id} role="listitem">
          <code>{item.id}</code>
          {item.source ? <span>{item.source}</span> : null}
        </span>
      ))}
    </span>
  )
}

/** Column 3 for the Agent access row: what the approval gate lets agents run on this server without a person,
 * and the one write that fixes it. The page reads the gate and probes it; this view only writes. */
export function AgentAccessView({
  iii,
  server,
  gate,
  verdicts,
  busy = false,
  onBusy,
  onChanged,
  onUseReadOnly,
}: {
  iii: ExtensionIii
  server: Server
  gate: GateRead
  verdicts: Record<string, Verdict> | undefined
  /** A save, approve or remove of the page runs: no rule write starts beside it. */
  busy?: boolean
  /** Raised around a rule write, so the page's own gate writes (Save, Approve, Remove) wait for it. */
  onBusy: (busy: boolean) => void
  onChanged: () => void
  onUseReadOnly: () => void
}) {
  const { confirm, dialog } = useConfirm()
  const [writing, setWriting] = useState<Write | null>(null)
  const [failure, setFailure] = useState<{ retry: Write; reason: string } | null>(null)
  const [settings, setSettings] = useState(false)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const access = serverAccess(server)
  const status = gateStatus({ gate, server, access, verdicts })

  if (status.kind === 'no_gate') {
    return (
      <div className="mcp-ui-detail mcp-ui-access">
        <StatusPanel
          variant="warn"
          headline="No approval gate on this stack."
          detail={coded(
            'Agents can call every function of this server, writes included, under the harness default allow `*`.',
          )}
        />
        <p className="mcp-ui-quiet-line">Install the approval-gate worker to hold writes for a person.</p>
        {readOnlyEndpoint(server.target) ? (
          <Button variant="secondary" size="sm" onClick={onUseReadOnly}>
            Use the read-only endpoint
          </Button>
        ) : null}
      </div>
    )
  }

  const lists = gate.rules ? accessLists(status, server.name, gate.rules, access, verdicts) : null
  const action = accessAction(status.kind)
  const alert = reviewAccessPanel(status, server.name)
  const listed = status.kind !== 'not_connected'

  const write = async (what: Write) => {
    if (writing || busy) return
    const allow = accessConfirm(server.name, status, access)
    const ask =
      what === 'remove'
        ? {
            title: `Remove the rules for ${server.name}?`,
            tone: 'danger' as const,
            confirmLabel: LABEL.remove,
            description: `Agents will have to ask before every call to ${server.name}.`,
          }
        : {
            title: allow.title,
            confirmLabel: LABEL[what],
            description: (
              <>
                <span className="mcp-ui-line">{allow.body}</span>
                {allow.groups.map((group) => (
                  <Fragment key={group.label}>
                    <b className="mcp-ui-line">{group.label}</b>
                    <Ids label={group.label} items={group.items} />
                  </Fragment>
                ))}
              </>
            ),
          }
    if (!(await confirm(ask))) return
    setWriting(what)
    setFailure(null)
    onBusy(true)
    try {
      const out = await writeBlock(iii, server.name, what === 'remove' ? [] : readIds(server))
      if (!out.ok) setFailure({ retry: what, reason: out.reason ?? '' })
      else {
        const text =
          what === 'remove'
            ? `Rules for ${server.name} removed from the approval gate`
            : `${out.written} ${out.written === 1 ? 'rule' : 'rules'} written to the approval gate`
        setAnnouncement((last) => ({ seq: (last?.seq ?? 0) + 1, text, urgency: 'polite' }))
      }
    } catch (err) {
      setFailure({ retry: what, reason: errorMessage(err) })
    } finally {
      setWriting(null)
      onBusy(false)
      // Written or not, the page re-reads the gate: the view shows the rules as stored.
      onChanged()
    }
  }
  // Remove rules is never a default button (spec 6.5): an outline in the danger tone in every state.
  const writeButton = (what: Write, primary: boolean) => (
    <Button
      variant={what === 'remove' ? 'secondary' : primary ? 'primary' : 'ghost'}
      className={what === 'remove' ? 'mcp-ui-danger' : undefined}
      size="sm"
      disabled={writing !== null || busy}
      onClick={() => void write(what)}
    >
      {writing === what ? 'Writing…' : LABEL[what]}
    </Button>
  )
  const removable = lists !== null && lists.block > 0
  const settingsButton =
    gate.id !== null ? (
      <Button variant="secondary" size="sm" onClick={() => setSettings(true)}>
        Open approval-gate settings
      </Button>
    ) : null

  return (
    <div className="mcp-ui-detail mcp-ui-access">
      <p className="mcp-ui-result-status">
        <StatusDot tone={status.tone} />
        {status.rowText}
      </p>
      {alert ? <StatusPanel variant="alert" headline={coded(alert)} /> : null}
      {status.kind === 'unreadable' ? <StatusPanel variant="alert" headline={gate.error ?? status.rowText} /> : null}
      {lists ? (
        <dl className="mcp-ui-counts">
          <div>
            <dt>Allowed without asking</dt>
            <dd>{lists.allowed.length}</dd>
          </div>
          {listed ? (
            <div>
              <dt>Ask a person</dt>
              <dd>{lists.ask.length}</dd>
            </div>
          ) : null}
          {listed && lists.askDestructive > 0 ? (
            <div data-tone="warn">
              <dt>Destructive, of those {lists.ask.length}</dt>
              <dd>{lists.askDestructive}</dd>
            </div>
          ) : null}
        </dl>
      ) : null}
      {lists && lists.allowed.length > 0 ? (
        <details className="mcp-ui-optional" open={!listed || undefined}>
          <summary>Allowed without asking</summary>
          <Ids items={lists.allowed} />
        </details>
      ) : null}
      {lists && lists.ask.length > 0 ? (
        <details className="mcp-ui-optional">
          <summary>Ask a person</summary>
          <Ids items={lists.ask} />
        </details>
      ) : null}
      {lists && lists.block > 0 && lists.toAdd.length > 0 ? (
        <details className="mcp-ui-optional" open>
          <summary>To add</summary>
          <Ids items={lists.toAdd} />
        </details>
      ) : null}
      {lists && lists.toRemove.length > 0 ? (
        <details className="mcp-ui-optional" open>
          <summary>To remove</summary>
          <Ids items={lists.toRemove} />
        </details>
      ) : null}
      {lists && lists.overridden.length > 0 ? (
        <details className="mcp-ui-optional" open>
          <summary>Overridden</summary>
          <Ids items={lists.overridden} />
        </details>
      ) : null}
      {failure ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline={failure.retry === 'remove' ? 'Could not remove the rules' : 'Could not write the rules'}
          detail={failure.reason}
          action={
            <>
              <Button
                variant="secondary"
                size="sm"
                disabled={writing !== null || busy}
                onClick={() => void write(failure.retry)}
              >
                Retry
              </Button>
              {entryRefused(failure.reason) ? settingsButton : null}
            </>
          }
        />
      ) : null}
      <div className="mcp-ui-run">
        {action === 'allow' || action === 'update' ? writeButton(action, true) : null}
        {action === 'remove' && removable ? writeButton('remove', true) : null}
        {action === 'settings' ? (
          <Button size="sm" onClick={() => setSettings(true)}>
            Open approval-gate settings
          </Button>
        ) : null}
        {action === 'retry' ? (
          <Button size="sm" onClick={onChanged}>
            Retry
          </Button>
        ) : null}
        {action === 'retry' ? settingsButton : null}
        {action !== 'remove' && removable ? writeButton('remove', false) : null}
      </div>
      <WorkerConfigurationDialog
        configurationId={settings ? gate.id : null}
        onClose={() => {
          setSettings(false)
          onChanged()
        }}
      />
      <LiveRegion announcement={announcement} />
      {dialog}
    </div>
  )
}
