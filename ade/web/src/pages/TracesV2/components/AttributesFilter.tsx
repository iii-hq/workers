// Inline editor for span-attribute filters, inside the Filters popover.
//
// A draft list of `key = value` rows. Rows edit freely; "Apply" (or Enter)
// commits the non-empty ones to the filter state. While the list is empty
// the common OTel keys sit below as one-click chips.

import { Check, Plus, X } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { Input } from '@/components/shadcn/input'

interface AttributesFilterProps {
  value: [string, string][]
  onChange: (attrs: [string, string][]) => void
}

const COMMON_ATTRIBUTES = [
  'http.request.method',
  'http.response.status_code',
  'http.route',
  'url.path',
  'code.file.path',
  'thread.name',
]

let _entryId = 0
type DraftEntry = { id: number; key: string; val: string }

const toDraftEntries = (pairs: [string, string][]): DraftEntry[] =>
  pairs.map(([key, val]) => ({ id: ++_entryId, key, val }))

const toValuePairs = (entries: DraftEntry[]): [string, string][] =>
  entries.map(({ key, val }) => [key, val])

export function AttributesFilter({ value, onChange }: AttributesFilterProps) {
  const [draft, setDraft] = useState<DraftEntry[]>(() => toDraftEntries(value))
  const [isDirty, setIsDirty] = useState(false)
  const [prevValue, setPrevValue] = useState(value)

  if (prevValue !== value) {
    setPrevValue(value)
    setDraft(toDraftEntries(value))
    setIsDirty(false)
  }

  const updateDraft = (newDraft: DraftEntry[]) => {
    setDraft(newDraft)
    setIsDirty(true)
  }

  const handleAdd = (key = '') => {
    updateDraft([...draft, { id: ++_entryId, key, val: '' }])
  }

  const handleRemove = (id: number) => {
    updateDraft(draft.filter((e) => e.id !== id))
  }

  const handleKeyChange = (id: number, key: string) => {
    updateDraft(draft.map((e) => (e.id === id ? { ...e, key } : e)))
  }

  const handleValueChange = (id: number, val: string) => {
    updateDraft(draft.map((e) => (e.id === id ? { ...e, val } : e)))
  }

  const handleApply = () => {
    const filtered = draft.filter(({ key }) => key.trim() !== '')
    onChange(toValuePairs(filtered))
    setIsDirty(false)
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && isDirty) {
      e.preventDefault()
      handleApply()
    }
  }

  return (
    <div className="flex flex-col gap-2" data-attributes-filter>
      {draft.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          {draft.map(({ id, key, val }) => (
            <div key={id} className="group flex items-center gap-1.5">
              <Input
                type="text"
                placeholder="key"
                value={key}
                aria-label="attribute key"
                onChange={(e) => handleKeyChange(id, e.target.value)}
                onKeyDown={handleKeyDown}
                className="h-7 flex-1 rounded-md px-2 font-mono text-sm"
              />
              <span className="shrink-0 font-mono text-xs text-muted-foreground">
                =
              </span>
              <Input
                type="text"
                placeholder="value"
                value={val}
                aria-label="attribute value"
                onChange={(e) => handleValueChange(id, e.target.value)}
                onKeyDown={handleKeyDown}
                className="h-7 flex-1 rounded-md px-2 font-mono text-sm"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                onClick={() => handleRemove(id)}
                title="Remove"
                aria-label="remove attribute"
                className="text-muted-foreground"
              >
                <X />
              </Button>
            </div>
          ))}
        </div>
      ) : null}

      <div className="flex flex-wrap items-center gap-1.5">
        <Button
          type="button"
          variant="outline"
          size="xs"
          onClick={() => handleAdd()}
          className="rounded-md"
        >
          <Plus data-icon="inline-start" />
          Add attribute
        </Button>
        {isDirty ? (
          <Button
            type="button"
            size="xs"
            onClick={handleApply}
            className="rounded-md"
          >
            <Check data-icon="inline-start" />
            Apply
          </Button>
        ) : null}
        {draft.length === 0 ? (
          <>
            <span className="ml-1 text-[11px] text-muted-foreground">
              Common:
            </span>
            {COMMON_ATTRIBUTES.map((attr) => (
              <button
                key={attr}
                type="button"
                onClick={() => handleAdd(attr)}
                className="h-6 rounded-md border border-border px-1.5 font-mono text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                title={`add ${attr}`}
              >
                {attr}
              </button>
            ))}
          </>
        ) : null}
      </div>
    </div>
  )
}
