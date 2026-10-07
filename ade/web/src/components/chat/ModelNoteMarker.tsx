import { AtSign, NotebookPen } from 'lucide-react'
import { useState } from 'react'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import { CardHighlight } from '@/components/ui/Surface'
import { cn } from '@/lib/utils'
import type { ModelNote, ModelNoteMention, SystemMessage } from '@/types/chat'
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
 * as the raw block.
 */
export function ModelNoteMarker({
  message,
  note,
}: {
  message: SystemMessage
  note: ModelNote
}) {
  const [open, setOpen] = useState(false)
  const mentions = note.mentions
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
          '@container trigger-activity-collapsible',
          !open && 'trigger-activity-collapsible--compact',
        )}
      >
        <CollapsibleCardTrigger
          className="group trigger-activity-collapsible__trigger select-none"
          aria-label={`${open ? 'Hide' : 'Show'} what the model was told`}
        >
          <div className="flex w-full min-w-0 items-center gap-2">
            {/* The kind glyph sits in the status slot, drawn like the Check
                a settled trigger shows: same box, size and stroke, so the
                row lines up with the trigger rows around it. */}
            <span
              aria-hidden="true"
              className="activity-status-icon"
              data-note-glyph={mentions ? 'mentions' : 'note'}
            >
              {mentions ? (
                <AtSign
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
            <p className="font-sans text-[12.5px] text-ink-faint">
              {mentions
                ? 'What the agent was told about the items this message mentions. It reads the full item only when the summary is not enough.'
                : 'Text the agent was shown at this point in the turn. Nothing to act on; it explains what the agent did next.'}
            </p>
            {mentions ? (
              <MentionsDetail mentions={mentions} />
            ) : (
              <CardHighlight className="p-3">
                <pre
                  data-model-note-text
                  className="max-h-80 overflow-auto whitespace-pre-wrap break-words font-mono text-[12px] leading-5 text-ink-faint"
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
                  <span className="font-sans text-[12px] text-warn">
                    {problem}
                  </span>
                ) : null}
              </div>
              {mention.status === 'resolved' && mention.summary ? (
                <p className="text-pretty wrap-break-word font-sans text-[12.5px] leading-5 text-ink-faint">
                  {mention.summary}
                </p>
              ) : null}
              {mention.details ? (
                <p
                  className="min-w-0 truncate font-mono text-[11px] leading-4 text-ink-ghost"
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
