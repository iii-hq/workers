import { AtSign, Brain, NotebookPen } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import { CardHighlight } from '@/components/ui/Surface'
import type { MemoryNote } from '@/lib/memory-note'
import { cn } from '@/lib/utils'
import type { ModelNote, ModelNoteMention, SystemMessage } from '@/types/chat'
import { MemoryList } from './MemoryList'
import { WorkerMentionPill } from './mentions/WorkerMentionPill'
import { TimelineActivityDisclosure } from './TimelineActivityTrail'

/** Pills shown on the collapsed row before "+N". */
const ROW_PILLS = 3

/**
 * Text the harness or a hook showed the model mid-turn, as the quietest row
 * in the timeline: the same one-line activity grammar as a function call or
 * the skill index, in faint ink, with what the model read behind the
 * disclosure. Nobody has to act on it; it is there to explain the reply.
 *
 * The judge's `<mentions>` note reads as the mentions themselves (pills on
 * the row; each item's summary and pre-verified details call inside), not
 * as the raw block. A memory note reads as what was recalled (the count on
 * the row, the memories inside), with the exact block one click away.
 */
export function ModelNoteMarker({
  message,
  note,
}: {
  message: SystemMessage
  note: ModelNote
}) {
  const [open, setOpen] = useState(false)
  const [exact, setExact] = useState(false)
  const mentions = note.mentions
  const memory = note.memory
  return (
    <article
      className="w-full"
      data-message-role="model-note"
      data-message-id={message.id}
      data-note-label={note.label}
      data-expanded={open}
    >
      <CollapsibleCard
        open={open}
        onOpenChange={setOpen}
        className={cn(
          '@container observer-activity-collapsible',
          !open && 'observer-activity-collapsible--compact',
        )}
      >
        <CollapsibleCardTrigger
          className="group observer-activity-collapsible__trigger select-none"
          aria-label={`${open ? 'Hide' : 'Show'} what the model was told`}
        >
          <div className="flex w-full min-w-0 items-center gap-2">
            {/* The kind glyph sits in the status slot, drawn like the Check
 a settled call shows: same box, size and stroke, so the
 row lines up with the activity rows around it. */}
            <span
              aria-hidden="true"
              className="activity-status-icon"
              data-note-glyph={
                mentions ? 'mentions' : memory ? 'memory' : 'note'
              }
            >
              {mentions ? (
                <AtSign
                  strokeWidth={2.5}
                  className="size-4 stroke-muted-foreground"
                />
              ) : memory ? (
                <Brain
                  strokeWidth={2.5}
                  className="size-4 stroke-muted-foreground"
                />
              ) : (
                <NotebookPen
                  strokeWidth={2.5}
                  className="size-4 stroke-muted-foreground"
                />
              )}
            </span>
            <div
              data-message-summary
              className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden font-sans text-[0.8125rem] text-muted-foreground"
            >
              {mentions ? (
                <MentionsSummary mentions={mentions} />
              ) : memory ? (
                <MemorySummary memory={memory} />
              ) : (
                <span className="truncate">
                  Note to the model
                  {note.label ? (
                    <span className="text-ink-ghost"> · {note.label}</span>
                  ) : null}
                </span>
              )}
            </div>
            <TimelineActivityDisclosure />
          </div>
        </CollapsibleCardTrigger>

        <CollapsibleCardContent>
          <div className="flex flex-col gap-3 border-t border-edge p-3">
            {memory ? (
              <div className="flex items-start justify-between gap-3">
                <div className="flex min-w-0 flex-col gap-0.5">
                  <p className="font-sans text-[12.5px] text-ink-faint">
                    {memory.rulesUpdate !== null
                      ? 'The memory rules the agent follows from this point on.'
                      : 'What the agent read from memory at this point in the turn.'}
                  </p>
                  <MemoryMeta memory={memory} />
                </div>
                <Button
                  variant="ghost"
                  size="xs"
                  aria-pressed={exact}
                  onClick={() => setExact((v) => !v)}
                  className="shrink-0 text-ink-faint aria-pressed:bg-surface-selected aria-pressed:text-ink"
                >
                  Exact text
                </Button>
              </div>
            ) : (
              <p className="font-sans text-[12.5px] text-ink-faint">
                {mentions
                  ? 'What the agent was told about the items this message mentions. It reads the full item only when the summary is not enough.'
                  : 'Text the agent was shown at this point in the turn. Nothing to act on; it explains what the agent did next.'}
              </p>
            )}
            {mentions ? (
              <MentionsDetail mentions={mentions} />
            ) : memory && !exact ? (
              <MemoryDetail memory={memory} />
            ) : (
              <CardHighlight className="p-3">
                <pre
                  data-model-note-text
                  className="max-h-80 overflow-auto whitespace-pre-wrap break-words font-mono text-xs leading-5 text-ink-faint"
                >
                  {note.text.trim()}
                </pre>
              </CardHighlight>
            )}
          </div>
        </CollapsibleCardContent>
      </CollapsibleCard>
    </article>
  )
}

function MentionsSummary({ mentions }: { mentions: ModelNoteMention[] }) {
  const shown = mentions.slice(0, ROW_PILLS)
  const more = mentions.length - shown.length
  return (
    <>
      <span className="shrink-0">Context for the agent</span>
      {shown.map((mention) => (
        <span
          key={`${mention.name}\u0000${mention.id}`}
          className="flex min-w-0 shrink items-center"
        >
          <WorkerMentionPill
            name={mention.name}
            id={mention.id}
            hoverCard={false}
          />
        </span>
      ))}
      {more > 0 ? <span className="shrink-0">+{more}</span> : null}
    </>
  )
}

const STATUS_COPY: Record<ModelNoteMention['status'], string | null> = {
  resolved: null,
  'not-found': 'not found by its worker',
  'unknown-provider': 'no installed worker provides it',
  error: 'could not be resolved',
}

function MentionsDetail({ mentions }: { mentions: ModelNoteMention[] }) {
  return (
    <CardHighlight className="p-3">
      <ul data-model-note-mentions className="flex flex-col gap-3">
        {mentions.map((mention) => {
          const problem = STATUS_COPY[mention.status]
          return (
            <li
              key={`${mention.name}\u0000${mention.id}`}
              className="flex min-w-0 flex-col gap-1"
            >
              <div className="flex min-w-0 items-center gap-2">
                <WorkerMentionPill
                  name={mention.name}
                  id={mention.id}
                  openOnClick
                />
                {problem ? (
                  <span className="font-sans text-xs text-warn">{problem}</span>
                ) : null}
              </div>
              {mention.status === 'resolved' && mention.summary ? (
                <p className="text-pretty wrap-break-word font-sans text-[12.5px] leading-5 text-ink-faint">
                  {mention.summary}
                </p>
              ) : null}
              {mention.details ? (
                <p
                  className="min-w-0 truncate font-mono text-meta leading-4 text-ink-ghost"
                  title={mention.details}
                >
                  details call · {mention.details}
                </p>
              ) : null}
            </li>
          )
        })}
      </ul>
    </CardHighlight>
  )
}

function memoryCount(n: number) {
  return `${n} ${n === 1 ? 'memory' : 'memories'}`
}

function MemorySummary({ memory }: { memory: MemoryNote }) {
  if (memory.rulesUpdate !== null) {
    return <span className="truncate">Memory rules updated</span>
  }
  const lead =
    memory.memories.length > 0
      ? `Recalled ${memoryCount(memory.memories.length)}`
      : 'Memory rules reminder'
  const tail = [
    memory.bank,
    memory.memories.length > 0 && memory.reminder !== null
      ? 'rules reminder'
      : null,
  ].filter(Boolean)
  return (
    <span className="truncate">
      {lead}
      {tail.length > 0 ? (
        <span className="text-ink-ghost"> · {tail.join(' · ')}</span>
      ) : null}
    </span>
  )
}

/** The day the recall ran: a machine value, in mono. */
function MemoryMeta({ memory }: { memory: MemoryNote }) {
  // The bank is on the row already; the day dates "last stated" below.
  if (!memory.recalled) return null
  return (
    <p className="truncate font-mono text-meta leading-4 text-ink-ghost">
      recalled {memory.recalled}
    </p>
  )
}

function MemoryDetail({ memory }: { memory: MemoryNote }) {
  const rules = memory.rulesUpdate ?? memory.reminder
  return (
    <CardHighlight className="flex flex-col gap-4 p-3">
      {memory.memories.length > 0 ? (
        <MemoryList
          items={memory.memories.map((m, i) => ({
            key: `${i}`,
            text: m.text,
            date: m.date,
          }))}
        />
      ) : null}
      {rules ? (
        <section className="flex flex-col gap-1.5">
          {memory.memories.length > 0 ? (
            <h4 className="font-sans text-xs font-medium text-ink-faint">
              Rules repeated with this reply
            </h4>
          ) : null}
          <p className="whitespace-pre-wrap text-pretty wrap-break-word font-sans text-[12.5px] leading-5 text-ink-faint">
            {rules}
          </p>
        </section>
      ) : null}
    </CardHighlight>
  )
}
