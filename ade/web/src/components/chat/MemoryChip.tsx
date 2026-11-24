import { Brain, ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/shadcn/popover'
import { Skeleton } from '@/components/shadcn/skeleton'
import { BottomSheet, BottomSheetContent } from '@/components/ui/BottomSheet'
import { useMediaQuery } from '@/hooks/use-media-query'
import { getMemory, type MemoryItem } from '@/lib/memory'
import { cn } from '@/lib/utils'
import { MemoryList } from './MemoryList'

/**
 * The "what fed this reply" chip on assistant messages: how many memories
 * the memory worker injected into the generate, and from which bank. It
 * opens a popover (a bottom sheet on phones) with the exact memories,
 * fetched by id on first open, so the header row and the reply never move.
 */

interface MemoryChipProps {
  memory: {
    bank: string
    memories: number
    memoryIds: string[]
    memoryBanks: string[]
    rules?: number
    truncated?: boolean
    semantic?: boolean
  }
}

export type MemoryLoad =
  | { state: 'idle' | 'loading' | 'error' }
  | { state: 'done'; memories: MemoryItem[] }

export function MemoryChip({ memory }: MemoryChipProps) {
  const [open, setOpen] = useState(false)
  const [load, setLoad] = useState<MemoryLoad>({ state: 'idle' })
  const phone = useMediaQuery('(max-width: 767px)')

  const counts = []
  if (memory.memories > 0) counts.push(plural(memory.memories, 'memory'))
  if (memory.rules) counts.push(plural(memory.rules, 'rule'))
  if (counts.length === 0) return null

  const fetchMemories = async () => {
    if (memory.memoryIds.length === 0) {
      setLoad({ state: 'done', memories: [] })
      return
    }
    setLoad({ state: 'loading' })
    try {
      // Each id from the bank holding it: a layer's live in another.
      const loaded = await Promise.all(
        memory.memoryIds.map((id, i) =>
          getMemory(memory.memoryBanks[i] ?? memory.bank, id),
        ),
      )
      setLoad({
        state: 'done',
        memories: loaded.filter((m): m is MemoryItem => m !== null),
      })
    } catch {
      // A failed fetch is not "no memories": say so, and let a retry run.
      setLoad({ state: 'error' })
    }
  }

  const onOpenChange = (next: boolean) => {
    setOpen(next)
    if (next && (load.state === 'idle' || load.state === 'error')) {
      void fetchMemories()
    }
  }

  const label = `Memory fed this reply: ${counts.join(', ')} from ${memory.bank}`
  const chipClass =
    'inline-flex h-5 min-w-0 max-w-full items-center gap-1.5 rounded-full bg-surface pr-1 pl-1.5 font-sans text-xs text-ink-faint outline-none transition-colors duration-[var(--motion-duration-control)] hover:bg-surface-hover hover:text-ink focus-visible:ring-3 focus-visible:ring-ring/50 aria-expanded:bg-surface-selected aria-expanded:text-ink'
  const chip = (
    <>
      <Brain aria-hidden className="size-4 shrink-0" />
      <span className="shrink-0 tabular-nums">{counts.join(' · ')}</span>
      <span className="min-w-0 truncate font-mono text-meta text-ink-ghost">
        {memory.bank}
      </span>
      <ChevronDown
        aria-hidden
        className={cn(
          'size-4 shrink-0 text-ink-ghost transition-transform duration-[var(--motion-duration-control)]',
          open && 'rotate-180',
        )}
      />
    </>
  )
  const body = (
    <MemoryChipBody
      load={load}
      rules={memory.rules ?? 0}
      retry={() => void fetchMemories()}
    />
  )

  if (phone) {
    return (
      <>
        <button
          type="button"
          data-memory-chip
          aria-label={label}
          aria-expanded={open}
          onClick={() => onOpenChange(!open)}
          className={chipClass}
        >
          {chip}
        </button>
        <BottomSheet open={open} onOpenChange={onOpenChange}>
          <BottomSheetContent
            heading="What memory fed this reply"
            description={<MemoryMeta {...memory} />}
            closeLabel="Close memories"
          >
            <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-4 pb-4">
              {body}
            </div>
          </BottomSheetContent>
        </BottomSheet>
      </>
    )
  }

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger data-memory-chip aria-label={label} className={chipClass}>
        {chip}
      </PopoverTrigger>
      <PopoverContent
        align="start"
        sideOffset={6}
        collisionPadding={12}
        aria-label="Memories that fed this reply"
        className="max-h-[min(28rem,var(--available-height))] w-[min(26rem,calc(100vw-24px))] gap-3 overflow-y-auto overscroll-contain p-3"
      >
        <MemoryChipPanel
          bank={memory.bank}
          semantic={memory.semantic}
          truncated={memory.truncated}
          rules={memory.rules ?? 0}
          load={load}
          retry={() => void fetchMemories()}
        />
      </PopoverContent>
    </Popover>
  )
}

/** The bank and how it was recalled: machine values, in mono. */
function MemoryMeta({
  bank,
  semantic,
  truncated,
}: {
  bank: string
  semantic?: boolean
  truncated?: boolean
}) {
  const meta = [
    bank,
    semantic ? 'semantic recall' : null,
    truncated ? 'cut to the recall budget' : null,
  ].filter(Boolean)
  return (
    <span className="truncate font-mono text-meta leading-4 text-ink-ghost">
      {meta.join(' · ')}
    </span>
  )
}

/** The popover's content, apart from its fetch so every state renders. */
export function MemoryChipPanel({
  bank,
  semantic,
  truncated,
  rules,
  load,
  retry,
}: {
  bank: string
  semantic?: boolean
  truncated?: boolean
  rules: number
  load: MemoryLoad
  retry: () => void
}) {
  return (
    <>
      <header className="flex min-w-0 flex-col gap-0.5">
        <h3 className="font-sans text-[13px] font-medium text-ink">
          What memory fed this reply
        </h3>
        <MemoryMeta bank={bank} semantic={semantic} truncated={truncated} />
      </header>
      <MemoryChipBody load={load} rules={rules} retry={retry} />
    </>
  )
}

function MemoryChipBody({
  load,
  rules,
  retry,
}: {
  load: MemoryLoad
  rules: number
  retry: () => void
}) {
  const rulesLine =
    rules > 0 ? (
      <p className="font-sans text-xs text-ink-faint">
        Plus {plural(rules, 'rule')} from the bank, sent on every turn.
      </p>
    ) : null

  if (load.state !== 'done' && load.state !== 'error') {
    return (
      <div aria-busy="true" className="flex flex-col gap-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="flex flex-col gap-1.5">
            <Skeleton className="h-3.5 w-full" />
            <Skeleton className="h-3.5 w-2/3" />
          </div>
        ))}
      </div>
    )
  }
  if (load.state !== 'done') {
    return (
      <div
        role="alert"
        className="flex items-center justify-between gap-3 rounded-lg bg-alert-muted py-1.5 pr-1.5 pl-3"
      >
        <p className="font-sans text-xs text-alert">
          Could not load the memories.
        </p>
        <Button variant="ghost" size="xs" onClick={retry}>
          Retry
        </Button>
      </div>
    )
  }
  if (load.memories.length === 0) {
    return (
      <>
        <p className="font-sans text-xs text-ink-faint">
          {rules > 0
            ? 'No memories were recalled for this reply.'
            : 'These memories are gone: superseded, deleted or moved to another bank.'}
        </p>
        {rulesLine}
      </>
    )
  }
  return (
    <>
      <MemoryList
        items={load.memories.map((m) => ({
          key: m.id,
          text: m.text,
          date: statedDate(m.stated_at ?? m.updated_at),
          pinned: m.pinned,
        }))}
      />
      {rulesLine}
    </>
  )
}

function plural(n: number, word: 'memory' | 'rule') {
  const words = word === 'memory' ? 'memories' : 'rules'
  return `${n} ${n === 1 ? word : words}`
}

/** `YYYY-MM-DD` in local time, as the memory worker dates its lines. */
function statedDate(ms: number) {
  return Number.isFinite(ms) && ms > 0
    ? new Date(ms).toLocaleDateString('sv-SE')
    : null
}
