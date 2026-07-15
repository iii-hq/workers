import { Plus } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { Input } from '@/components/ui/Input'
import type { MemoryBlock } from '@/lib/memory'

/**
 * The bank's markdown blocks — injected whole into the system prompt of
 * every session using this bank. Each block is a plain `.md` file on disk;
 * editing here and editing the file are equivalent. Saving empty content
 * removes a block.
 */

interface BlocksPanelProps {
  blocks: MemoryBlock[]
  onSet: (name: string, content: string) => void
  busy: boolean
}

function BlockEditor({
  name,
  initial,
  onSet,
  busy,
}: {
  name: string
  initial: string
  onSet: (name: string, content: string) => void
  busy: boolean
}) {
  const [content, setContent] = useState(initial)
  // Re-seed the editor when a live refresh changes the block on disk and
  // the user has no local edits in flight.
  const [touched, setTouched] = useState(false)
  useEffect(() => {
    if (!touched) setContent(initial)
  }, [initial, touched])

  const dirty = content !== initial
  return (
    <div className="border border-rule">
      <div className="flex items-center justify-between px-3 py-1.5 border-b border-rule-2">
        <span className="font-mono text-[12px] lowercase text-ink font-semibold">
          {name}.md
        </span>
        <div className="flex items-center gap-2">
          {dirty ? (
            <span className="font-mono text-[10px] lowercase text-warn">
              unsaved
            </span>
          ) : null}
          <Button
            variant="ghost"
            size="sm"
            disabled={!dirty || busy}
            onClick={() => {
              onSet(name, content)
              setTouched(false)
            }}
          >
            save
          </Button>
        </div>
      </div>
      <textarea
        value={content}
        onChange={(e) => {
          setContent(e.target.value)
          setTouched(true)
        }}
        rows={Math.min(24, Math.max(4, content.split('\n').length + 1))}
        spellCheck={false}
        aria-label={`rule ${name}`}
        className="w-full bg-bg text-ink font-mono text-[13px] leading-relaxed p-3 outline-none resize-y placeholder:text-ink-ghost"
        placeholder="empty content removes this rule on save"
      />
    </div>
  )
}

export function BlocksPanel({ blocks, onSet, busy }: BlocksPanelProps) {
  const [newName, setNewName] = useState('')
  const validName = /^[a-z0-9][a-z0-9_-]{0,63}$/.test(newName)

  return (
    <div className="flex flex-col gap-3">
      <p className="font-mono text-[11px] lowercase text-ink-faint">
        rules go into the system prompt on EVERY turn, guaranteed — unlike
        memories, which are recalled only when they match the question. use
        rules for what must always hold: writing style, answer format, coding
        conventions, project constants. each rule is a markdown file under the
        bank's blocks/ folder (editing the file and editing here are the same
        thing), and the agent updates rules as you correct it.
      </p>
      {blocks.length === 0 ? (
        <EmptyState
          title="no rules in this bank"
          description="try one: add a rule named `assistant-rules` with 'always answer tersely, prefer code examples' — then ask anything in chat and watch every reply obey it, on any topic."
        />
      ) : (
        blocks.map((block) => (
          <BlockEditor
            key={block.name}
            name={block.name}
            initial={block.content}
            onSet={onSet}
            busy={busy}
          />
        ))
      )}
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          if (!validName || busy) return
          onSet(newName, `# ${newName}\n`)
          setNewName('')
        }}
      >
        <Input
          value={newName}
          onChange={setNewName}
          placeholder="new rule name (e.g. style)"
          aria-label="new rule name"
          className="flex-1"
        />
        <Button
          type="submit"
          variant="ghost"
          size="sm"
          disabled={!validName || busy}
          className="gap-1"
        >
          <Plus className="w-3.5 h-3.5" aria-hidden />
          add rule
        </Button>
      </form>
    </div>
  )
}
