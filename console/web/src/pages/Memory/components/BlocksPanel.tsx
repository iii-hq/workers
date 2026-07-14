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
        rows={Math.max(4, content.split('\n').length + 1)}
        spellCheck={false}
        aria-label={`block ${name}`}
        className="w-full bg-bg text-ink font-mono text-[13px] leading-relaxed p-3 outline-none resize-y placeholder:text-ink-ghost"
        placeholder="empty content removes this block on save"
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
        blocks are injected whole into the system prompt on every turn using
        this bank — durable, identity-grade guidance (style, preferences,
        standing instructions)
      </p>
      {blocks.length === 0 ? (
        <EmptyState
          title="no blocks in this bank"
          description="add one below — e.g. a `style` block with writing rules. blocks are markdown files under the bank's blocks/ folder."
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
          placeholder="new block name (e.g. style)"
          aria-label="new block name"
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
          add block
        </Button>
      </form>
    </div>
  )
}
