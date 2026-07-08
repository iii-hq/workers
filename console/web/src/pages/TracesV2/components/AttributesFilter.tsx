// Inline editor for span-attribute filters.
//
// The editor itself isn't collapsible — TraceFilters wraps the whole
// affordance in a `<details className="iii-details">` so the open/close
// chrome is handled at the parent. This keeps the editor focused on the
// "draft + apply" loop. Pressing Enter from either input applies the
// staged draft.

import { Check, Plus, Tag, X } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/Button'

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
  'code.module.name',
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

  const handleAdd = () => {
    updateDraft([...draft, { id: ++_entryId, key: '', val: '' }])
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

  const handleSuggestionClick = (key: string) => {
    updateDraft([...draft, { id: ++_entryId, key, val: '' }])
  }

  const handleApply = () => {
    const filtered = draft.filter(({ key }) => key.trim() !== '')
    onChange(toValuePairs(filtered))
    setIsDirty(false)
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && isDirty) {
      handleApply()
    }
  }

  return (
    <div className="space-y-2">
      {draft.length === 0 ? (
        <div className="font-mono text-[12px] text-ink-faint italic lowercase">
          filter by span attributes (e.g. http.request.method = post)
        </div>
      ) : (
        <div className="space-y-2">
          {draft.map(({ id, key, val }) => (
            <div
              key={id}
              className="group flex items-center gap-2 bg-bg border border-rule-2 p-2 hover:border-rule transition-colors"
            >
              <input
                type="text"
                placeholder="key"
                value={key}
                onChange={(e) => handleKeyChange(id, e.target.value)}
                onKeyDown={handleKeyDown}
                className="flex-1 bg-transparent border-none font-mono text-[12px] text-ink placeholder:text-ink-faint focus:outline-none lowercase"
              />
              <span className="text-ink-faint font-mono text-[12px]">=</span>
              <input
                type="text"
                placeholder="value"
                value={val}
                onChange={(e) => handleValueChange(id, e.target.value)}
                onKeyDown={handleKeyDown}
                className="flex-1 bg-transparent border-none font-mono text-[12px] text-ink placeholder:text-ink-faint focus:outline-none lowercase"
              />
              <button
                type="button"
                onClick={() => handleRemove(id)}
                className="p-1 text-ink-faint hover:text-alert hover:bg-panel transition-all opacity-0 group-hover:opacity-100"
                title="remove"
                aria-label="remove attribute"
              >
                <X className="w-3 h-3" />
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="flex items-center justify-between pt-1 flex-wrap gap-2">
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={handleAdd}
            className="text-accent"
          >
            <Plus className="w-3 h-3" />
            add
          </Button>

          {isDirty && (
            <Button variant="pill" size="sm" onClick={handleApply}>
              <Check className="w-3 h-3" />
              apply
            </Button>
          )}
        </div>

        {draft.length === 0 && (
          <div className="flex items-center gap-1 flex-wrap">
            {COMMON_ATTRIBUTES.map((attr) => (
              <button
                key={attr}
                type="button"
                onClick={() => handleSuggestionClick(attr)}
                className="flex items-center gap-1 px-1.5 py-0.5 font-mono text-[10px] text-ink-faint bg-bg border border-rule-2 hover:border-accent hover:text-accent transition-colors lowercase"
                title={`add ${attr}`}
              >
                <Tag className="w-2.5 h-2.5" />
                {attr}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
