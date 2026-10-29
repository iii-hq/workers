/**
 * Sessions → windows → sub-agents, with what runs in each window: the agent
 * in its panes and its state, the sub-agents those agents have at work, an
 * `iii compose` stack and its engine port, and the ports the window's
 * processes listen on. Rows follow `uiClasses.tree*`; the
 * current session (the one this page's terminal is on) and its current
 * window carry the neutral selected treatment.
 */

import { Chip, StatusDot, uiClasses } from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { AppWindow, Bell, Bot, ChevronRight, Monitor, Pencil, Plus, SquareTerminal, Trash2 } from 'lucide-react'
import { type CSSProperties, Fragment, type KeyboardEvent, type ReactNode, useEffect, useState } from 'react'
import {
  type AgentBadge,
  agentStateLabel,
  agentStateTone,
  type SessionRow,
  type SubagentRow,
  subagentCountLabel,
  type WindowRow,
} from '../lib/model'

const MAX_PORT_CHIPS = 2

export interface SessionTreeProps {
  rows: SessionRow[]
  narrow: boolean
  /** Key of the sub-agent whose transcript the page shows. */
  selectedSubagent?: string | null
  onSelectSession(row: SessionRow): void
  onSelectWindow(session: SessionRow, window: WindowRow): void
  onSelectSubagent(sub: SubagentRow): void
  onNewWindow(session: SessionRow): void
  onRenameSession(session: SessionRow): void
  onKillSession(session: SessionRow): void
  onRenameWindow(session: SessionRow, window: WindowRow): void
  onKillWindow(session: SessionRow, window: WindowRow): void
}

function activateOnKey(event: KeyboardEvent<HTMLElement>, run: () => void) {
  if (event.target !== event.currentTarget) return
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault()
    run()
  }
}

function RowAction({
  label,
  tone,
  pointer,
  onClick,
  children,
}: {
  label: string
  tone?: 'alert'
  pointer?: 'fine'
  onClick(): void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      className={uiClasses.treeItemAction}
      aria-label={label}
      title={label}
      data-tone={tone}
      data-pointer={pointer}
      onClick={(event) => {
        event.stopPropagation()
        onClick()
      }}
    >
      {children}
    </button>
  )
}

export function agentSummary(agent: AgentBadge, now = Date.now()): string {
  const since = formatRelative(agent.sinceMs, now)
  const subagents = agent.subagents.length > 0 ? ` · ${subagentCountLabel(agent.subagents.length)}` : ''
  return `${agent.label} · ${agentStateLabel(agent.state)}${since ? ` for ${since}` : ''}${subagents}`
}

export function subagentSummary(sub: SubagentRow, now = Date.now()): string {
  const since = formatRelative(sub.sinceMs, now)
  const kind = sub.kind ? ` (${sub.kind})` : ''
  return `${sub.description}${kind} · ${sub.agentLabel} sub-agent${since ? `, working for ${since}` : ''}`
}

function windowTitle(row: WindowRow): string {
  const lines = [`${row.sessionName}:${row.label}`]
  for (const pane of row.window.panes) {
    const running = pane.agent?.subagents?.length ?? 0
    const subagents = running > 0 ? `, ${subagentCountLabel(running)}` : ''
    const what = pane.agent ? `${pane.agent.label} (${agentStateLabel(pane.agent.state)}${subagents})` : pane.command
    const ports = pane.ports.length > 0 ? ` · ${pane.ports.map((p) => `:${p.port}`).join(' ')}` : ''
    lines.push(`${pane.id} ${what} — ${pane.cwd}${ports}`)
  }
  return lines.join('\n')
}

/** `Date.now()`, refreshed every `everyMs` while `active`, for elapsed labels. */
function useNow(active: boolean, everyMs = 15_000): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    setNow(Date.now())
    const timer = setInterval(() => setNow(Date.now()), everyMs)
    return () => clearInterval(timer)
  }, [active, everyMs])
  return now
}

function WindowBadges({ row }: { row: WindowRow }) {
  const extraPorts = row.ports.length - MAX_PORT_CHIPS
  return (
    <>
      {row.agents.map((agent) => (
        <span key={agent.paneId} className="tmux-agent" title={agentSummary(agent)}>
          <StatusDot tone={agentStateTone(agent.state)} pulse={agent.state === 'working'} />
          <span className="tmux-agent-label">{agent.id}</span>
          {agent.subagents.length > 0 ? (
            <span className="tmux-agent-subagents">
              <Bot className="tmux-meta-icon" aria-hidden />
              {agent.subagents.length}
            </span>
          ) : null}
          {agent.state === 'waiting' ? <span className="tmux-agent-waiting">waiting</span> : null}
        </span>
      ))}
      {row.compose ? (
        <Chip className="tmux-chip" tone="neutral" title={`iii compose · namespace ${row.compose.namespace}`}>
          compose{row.compose.wsPort !== null ? ` · :${row.compose.wsPort}` : ''}
        </Chip>
      ) : null}
      {row.compose
        ? null
        : row.ports.slice(0, MAX_PORT_CHIPS).map((port) => (
            <Chip key={port} className="tmux-chip tmux-mono" tone="neutral">
              :{port}
            </Chip>
          ))}
      {!row.compose && extraPorts > 0 ? <span className={uiClasses.treeItemMeta}>+{extraPorts}</span> : null}
    </>
  )
}

function SessionMeta({ row }: { row: SessionRow }) {
  return (
    <>
      {row.agents.waiting > 0 ? (
        <span className="tmux-count" title={`${row.agents.waiting} waiting for you`}>
          <StatusDot tone="warn" />
          <span className={uiClasses.treeItemMeta}>{row.agents.waiting}</span>
        </span>
      ) : null}
      {row.agents.working > 0 ? (
        <span
          className="tmux-count"
          title={`${row.agents.working} working${row.agents.subagents > 0 ? `, with ${subagentCountLabel(row.agents.subagents)}` : ''}`}
        >
          <StatusDot tone="accent" pulse />
          <span className={uiClasses.treeItemMeta}>{row.agents.working}</span>
        </span>
      ) : null}
      {row.externalClients > 0 ? (
        <span
          className="tmux-count"
          title={`${row.externalClients} terminal${row.externalClients === 1 ? '' : 's'} attached outside the console`}
        >
          <Monitor className="tmux-meta-icon" aria-hidden />
          <span className={uiClasses.treeItemMeta}>{row.externalClients}</span>
        </span>
      ) : null}
      {row.consoleClients > 0 ? (
        <Chip
          className="tmux-chip"
          tone="neutral"
          title={`${row.consoleClients} console terminal${row.consoleClients === 1 ? '' : 's'} attached`}
        >
          console
        </Chip>
      ) : null}
    </>
  )
}

/** A sub-agent at work, under its window; selecting it opens its transcript. */
function SubagentItem({
  sub,
  now,
  selected,
  onSelect,
}: {
  sub: SubagentRow
  now: number
  selected: boolean
  onSelect(): void
}) {
  const since = formatRelative(sub.sinceMs, now)
  return (
    // biome-ignore lint/a11y/useSemanticElements: tree rows share the treeItem recipe with the window rows.
    <div
      className={`${uiClasses.treeItem} tmux-subagent`}
      role="button"
      aria-current={selected ? 'true' : undefined}
      tabIndex={0}
      style={{ '--iii-ui-tree-depth': 1 + sub.depth } as CSSProperties}
      title={subagentSummary(sub, now)}
      onClick={onSelect}
      onKeyDown={(event) => activateOnKey(event, onSelect)}
    >
      <span className={uiClasses.treeItemIcon}>
        <Bot aria-hidden />
      </span>
      <span className={uiClasses.treeItemLabel}>{sub.description}</span>
      <span className={uiClasses.treeItemTrailing}>
        {sub.kind ? <span className="tmux-agent-label">{sub.kind}</span> : null}
        {since ? <span className={uiClasses.treeItemMeta}>{since}</span> : null}
        <StatusDot tone="accent" pulse />
      </span>
    </div>
  )
}

export function SessionTree({
  rows,
  narrow,
  onSelectSession,
  onSelectWindow,
  onNewWindow,
  onRenameSession,
  onKillSession,
  onRenameWindow,
  onKillWindow,
  selectedSubagent = null,
  onSelectSubagent,
}: SessionTreeProps) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set())
  const now = useNow(rows.some((row) => row.agents.subagents > 0))
  const toggle = (key: string) =>
    setCollapsed((current) => {
      const next = new Set(current)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })

  return (
    <div className={`${uiClasses.tree} tmux-tree`} data-narrow={narrow ? '' : undefined}>
      {rows.map((row) => {
        const open = !collapsed.has(row.key)
        return (
          <div key={row.key} className="tmux-tree-group">
            {/* biome-ignore lint/a11y/useSemanticElements: the row hosts nested caret and action buttons. */}
            <div
              className={uiClasses.treeItem}
              role="button"
              aria-current={row.current ? 'true' : undefined}
              tabIndex={0}
              style={{ '--iii-ui-tree-depth': 0 } as CSSProperties}
              title={`${row.session.name} · ${row.session.windows.length} window${row.session.windows.length === 1 ? '' : 's'}`}
              onClick={() => onSelectSession(row)}
              onKeyDown={(event) => activateOnKey(event, () => onSelectSession(row))}
            >
              <span className={uiClasses.treeItemIcon} data-color="teal">
                <SquareTerminal aria-hidden />
              </span>
              <span className={uiClasses.treeItemLabel}>{row.session.name}</span>
              <button
                type="button"
                className={uiClasses.treeItemCaret}
                aria-expanded={open}
                aria-label={open ? `Collapse ${row.session.name}` : `Expand ${row.session.name}`}
                onClick={(event) => {
                  event.stopPropagation()
                  toggle(row.key)
                }}
              >
                <ChevronRight aria-hidden />
              </button>
              <span className={uiClasses.treeItemTrailing}>
                <SessionMeta row={row} />
                <span className={uiClasses.treeItemActions}>
                  <RowAction label={`New window in ${row.session.name}`} onClick={() => onNewWindow(row)}>
                    <Plus aria-hidden />
                  </RowAction>
                  <RowAction label={`Rename ${row.session.name}`} pointer="fine" onClick={() => onRenameSession(row)}>
                    <Pencil aria-hidden />
                  </RowAction>
                  <RowAction label={`Kill session ${row.session.name}`} tone="alert" onClick={() => onKillSession(row)}>
                    <Trash2 aria-hidden />
                  </RowAction>
                </span>
              </span>
            </div>
            {open
              ? row.windows.map((windowRow) => (
                  <Fragment key={windowRow.key}>
                    {/* biome-ignore lint/a11y/useSemanticElements: the row hosts nested action buttons. */}
                    <div
                      className={uiClasses.treeItem}
                      role="button"
                      aria-current={row.current && windowRow.active ? 'true' : undefined}
                      tabIndex={0}
                      style={{ '--iii-ui-tree-depth': 1 } as CSSProperties}
                      title={windowTitle(windowRow)}
                      onClick={() => onSelectWindow(row, windowRow)}
                      onKeyDown={(event) => activateOnKey(event, () => onSelectWindow(row, windowRow))}
                    >
                      <span className={uiClasses.treeItemIcon}>
                        <AppWindow aria-hidden />
                      </span>
                      <span className={uiClasses.treeItemLabel}>
                        <span className="tmux-mono tmux-window-index">{windowRow.window.index}</span>
                        {windowRow.window.name}
                      </span>
                      {windowRow.active ? (
                        <span
                          className="tmux-active-mark"
                          role="img"
                          aria-label="current window"
                          title="Current window of the session"
                        />
                      ) : null}
                      <span className={uiClasses.treeItemTrailing}>
                        {windowRow.window.bell ? <Bell className="tmux-meta-icon tmux-bell" aria-label="Bell" /> : null}
                        <WindowBadges row={windowRow} />
                        <span className={uiClasses.treeItemActions}>
                          <RowAction
                            label={`Rename window ${windowRow.label}`}
                            pointer="fine"
                            onClick={() => onRenameWindow(row, windowRow)}
                          >
                            <Pencil aria-hidden />
                          </RowAction>
                          <RowAction
                            label={`Kill window ${windowRow.label}`}
                            tone="alert"
                            onClick={() => onKillWindow(row, windowRow)}
                          >
                            <Trash2 aria-hidden />
                          </RowAction>
                        </span>
                      </span>
                    </div>
                    {windowRow.subagents.map((sub) => (
                      <SubagentItem
                        key={sub.key}
                        sub={sub}
                        now={now}
                        selected={sub.key === selectedSubagent}
                        onSelect={() => onSelectSubagent(sub)}
                      />
                    ))}
                  </Fragment>
                ))
              : null}
          </div>
        )
      })}
    </div>
  )
}
