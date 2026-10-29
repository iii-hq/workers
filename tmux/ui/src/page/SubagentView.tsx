/**
 * One sub-agent's transcript: the task it was given, then what it wrote and
 * reasoned, the tools it called and what they returned, oldest first. A
 * sub-agent has no pane of its own, so this reads what its agent records
 * (`tmux::subagents::transcript`), every 2 s while it works. The list
 * follows new items while it is scrolled to the bottom.
 */

import {
  Badge,
  Chip,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  EmptyState,
  IconButton,
  StatusDot,
  StatusPanel,
  Toolbar,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Bot, Brain, CornerDownRight, MessageSquare, TriangleAlert, Wrench, X } from 'lucide-react'
import { type ComponentType, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { TmuxApi } from '../lib/api'
import { type SubagentRow, transcriptItemLabel, transcriptPollMs } from '../lib/model'
import type { SubagentTranscript, TranscriptItem, TranscriptItemKind } from '../lib/types'

const ICONS: Record<TranscriptItemKind, ComponentType<{ className?: string; 'aria-hidden'?: boolean }>> = {
  user: MessageSquare,
  text: Bot,
  thinking: Brain,
  tool_call: Wrench,
  tool_result: CornerDownRight,
}

/** Within this many pixels of the bottom, new items keep the list there. */
const STICK_PX = 48

interface TranscriptState {
  transcript: SubagentTranscript | null
  error: string | null
}

function useTranscript(api: TmuxApi, paneId: string, subagentId: string): TranscriptState {
  const [state, setState] = useState<TranscriptState>({ transcript: null, error: null })
  useEffect(() => {
    let cancelled = false
    let timer: number | undefined
    let last: SubagentTranscript | null = null
    setState({ transcript: null, error: null })
    const tick = async () => {
      let failed = false
      try {
        last = await api.subagentTranscript(paneId, subagentId)
        if (cancelled) return
        setState({ transcript: last, error: null })
      } catch (error) {
        if (cancelled) return
        failed = true
        setState((current) => ({ ...current, error: errorMessage(error) }))
      }
      timer = window.setTimeout(tick, transcriptPollMs(last, failed))
    }
    void tick()
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [api, paneId, subagentId])
  return state
}

function clock(atMs: number | null): string | null {
  return atMs === null ? null : new Date(atMs).toLocaleTimeString()
}

function Item({ item, agentLabel }: { item: TranscriptItem; agentLabel: string }) {
  const Icon = item.error ? TriangleAlert : ICONS[item.kind]
  const mono = item.kind === 'tool_call' || item.kind === 'tool_result'
  const time = clock(item.at_ms)
  return (
    <li className={`tmux-step tmux-step-${item.kind}${item.error ? ' tmux-step-error' : ''}`}>
      <div className="tmux-step-head">
        <Icon className="tmux-meta-icon" aria-hidden />
        <span className="tmux-step-label">{transcriptItemLabel(item, agentLabel)}</span>
        {time ? <span className="tmux-faint tmux-mono tmux-step-time">{time}</span> : null}
      </div>
      {item.text ? <div className={mono ? 'tmux-step-text tmux-step-code' : 'tmux-step-text'}>{item.text}</div> : null}
    </li>
  )
}

export interface SubagentViewProps {
  api: TmuxApi
  subagent: SubagentRow
  onClose(): void
}

export function SubagentView({ api, subagent, onClose }: SubagentViewProps) {
  const { transcript, error } = useTranscript(api, subagent.paneId, subagent.id)
  const scroller = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  const items = transcript?.items ?? []
  const lastKey = items.length > 0 ? `${items.length}:${items[items.length - 1].at_ms}` : ''

  // A new sub-agent opens at its latest step.
  useEffect(() => {
    stick.current = true
  }, [subagent.key])

  useLayoutEffect(() => {
    const node = scroller.current
    if (node && stick.current && lastKey) node.scrollTop = node.scrollHeight
  }, [lastKey])

  const working = transcript ? transcript.working : true
  return (
    <div className="tmux-subagent-view">
      <Toolbar
        aria-label="Sub-agent"
        className="tmux-terminal-toolbar"
        end={
          <IconButton label="Close the sub-agent" variant="ghost" onClick={onClose}>
            <X aria-hidden />
          </IconButton>
        }
      >
        <span className="tmux-subagent-title">
          <Bot className="tmux-meta-icon" aria-hidden />
          <span className="tmux-subagent-name">{subagent.description}</span>
          {subagent.kind ? <Chip>{subagent.kind}</Chip> : null}
          <span className="tmux-faint">{subagent.agentLabel}</span>
          {working ? (
            <Badge variant="accent">
              <StatusDot tone="accent" pulse /> working
            </Badge>
          ) : (
            <Badge>finished</Badge>
          )}
        </span>
      </Toolbar>
      <div
        ref={scroller}
        className="tmux-subagent-scroll"
        onScroll={(event) => {
          const node = event.currentTarget
          stick.current = node.scrollHeight - node.scrollTop - node.clientHeight < STICK_PX
        }}
      >
        {error && !transcript ? (
          <StatusPanel variant="alert" headline="Could not read the sub-agent's transcript" detail={error} />
        ) : !transcript ? (
          <p className="tmux-faint tmux-side-note">Reading the transcript…</p>
        ) : (
          <>
            {transcript.prompt ? (
              <CollapsibleCard className="tmux-subagent-prompt" defaultOpen={items.length === 0}>
                <CollapsibleCardTrigger>Task</CollapsibleCardTrigger>
                <CollapsibleCardContent>
                  <div className="tmux-step-text">{transcript.prompt}</div>
                </CollapsibleCardContent>
              </CollapsibleCard>
            ) : null}
            {transcript.truncated ? <p className="tmux-faint tmux-side-note">Earlier steps are not shown.</p> : null}
            {items.length === 0 ? (
              <EmptyState
                compact
                icon={Bot}
                title="No steps yet"
                description="The sub-agent has not written anything since its task."
              />
            ) : (
              <ol className="tmux-steps" aria-label={`Steps of ${subagent.description}`}>
                {items.map((item, index) => (
                  <Item key={`${index}:${item.at_ms}`} item={item} agentLabel={subagent.agentLabel} />
                ))}
              </ol>
            )}
            {error ? <p className="tmux-faint tmux-side-note">Not updating: {error}</p> : null}
          </>
        )}
      </div>
    </div>
  )
}
