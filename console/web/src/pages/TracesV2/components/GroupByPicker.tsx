// Group-by picker for the TRACES tab.
//
// Grouping is generic: `engine::traces::group_by` groups by ANY span
// attribute key, so the picker is presets (message / session / function —
// the identity keys iii stamps via baggage) on top of a free-form attribute
// input. Suggestions come from every attribute key observed on loaded rows
// (root-span attributes + merged trace tags), but any typed key works even
// if it hasn't been seen yet.

import { Check, ChevronDown, Layers, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { cn } from '@/lib/utils'
import { type GroupByOption, groupByLabel } from '../lib/groupTraces'

const PRESETS: Array<{ value: GroupByOption; label: string }> = [
  { value: 'none', label: 'no grouping' },
  { value: 'iii.message.id', label: 'message' },
  { value: 'iii.session.id', label: 'session' },
  { value: 'iii.function.id', label: 'function' },
]

const MAX_SUGGESTIONS = 8

interface GroupByPickerProps {
  value: GroupByOption
  onChange: (next: GroupByOption) => void
  /** Attribute keys observed on loaded traces, iii.* first. */
  suggestions: string[]
}

export function GroupByPicker({
  value,
  onChange,
  suggestions,
}: GroupByPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const triggerRef = useRef<HTMLButtonElement>(null)
  const popoverRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null)

  useEffect(() => {
    if (!open || !triggerRef.current) return
    const rect = triggerRef.current.getBoundingClientRect()
    setPos({ top: rect.bottom + 4, left: rect.left })
    setQuery('')
    // Focus after the portal mounts.
    requestAnimationFrame(() => inputRef.current?.focus())
  }, [open])

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (popoverRef.current?.contains(target)) return
      if (triggerRef.current?.contains(target)) return
      setOpen(false)
    }
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', handleClick)
    document.addEventListener('keydown', handleEscape)
    return () => {
      document.removeEventListener('mousedown', handleClick)
      document.removeEventListener('keydown', handleEscape)
    }
  }, [open])

  const apply = (next: GroupByOption) => {
    onChange(next)
    setOpen(false)
  }

  const trimmed = query.trim()
  const presetValues = new Set<string>(PRESETS.map((p) => p.value))
  const filtered = (
    trimmed
      ? suggestions.filter((k) =>
          k.toLowerCase().includes(trimmed.toLowerCase()),
        )
      : suggestions
  )
    .filter((k) => !presetValues.has(k))
    .slice(0, MAX_SUGGESTIONS)

  const isActive = value !== 'none'

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="dialog"
        aria-expanded={open}
        className={cn(
          'inline-flex items-center gap-2 h-8 px-2.5 border font-mono text-[12px] lowercase transition-colors',
          open || isActive
            ? 'border-accent text-ink'
            : 'border-rule text-ink-faint hover:text-ink',
        )}
      >
        <Layers className="w-3 h-3" />
        <span className="max-w-[200px] truncate">{groupByLabel(value)}</span>
        {isActive && (
          // Nested interactive inside the trigger button; the keyboard path
          // is the "no grouping" row in the popover, hence tabIndex -1.
          // biome-ignore lint/a11y/useSemanticElements: nested clear affordance inside the trigger button
          <span
            role="button"
            tabIndex={-1}
            aria-label="clear grouping"
            onClick={(e) => {
              e.stopPropagation()
              onChange('none')
            }}
            onKeyDown={(e) => {
              if (e.key !== 'Enter' && e.key !== ' ') return
              e.preventDefault()
              e.stopPropagation()
              onChange('none')
            }}
            className="text-ink-faint hover:text-ink transition-colors"
          >
            <X className="w-3 h-3" />
          </span>
        )}
        <ChevronDown
          className={cn('w-3 h-3 transition-transform', open && 'rotate-180')}
        />
      </button>

      {open &&
        pos &&
        createPortal(
          <div
            ref={popoverRef}
            style={{
              position: 'fixed',
              top: pos.top,
              left: pos.left,
              zIndex: 50,
            }}
            className="w-[280px] bg-bg border border-rule shadow-[0_8px_24px_rgba(0,0,0,0.18)]"
          >
            <div className="p-1">
              {PRESETS.map((preset) => (
                <button
                  key={preset.value}
                  type="button"
                  onClick={() => apply(preset.value)}
                  className={cn(
                    'w-full flex items-center gap-2 px-2 py-1.5 text-left font-mono text-[12px] lowercase transition-colors hover:bg-panel',
                    preset.value === value ? 'text-ink' : 'text-ink-faint',
                  )}
                >
                  <span className="w-3">
                    {preset.value === value && <Check className="w-3 h-3" />}
                  </span>
                  {preset.label}
                </button>
              ))}
            </div>
            <div className="border-t border-rule-2 p-2 space-y-1">
              <div className="font-mono text-[10px] text-ink-faint uppercase tracking-[0.06em]">
                any attribute
              </div>
              <input
                ref={inputRef}
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key !== 'Enter') return
                  const next = trimmed || filtered[0]
                  if (next) apply(next)
                }}
                placeholder="e.g. iii.tag.message, service.name"
                className="w-full h-8 px-2 font-mono text-[12px] bg-bg border border-rule text-ink placeholder:text-ink-ghost focus:outline-none focus:border-accent transition-colors lowercase"
              />
              <div>
                {filtered.map((key) => (
                  <button
                    key={key}
                    type="button"
                    onClick={() => apply(key)}
                    className={cn(
                      'w-full flex items-center gap-2 px-2 py-1 text-left font-mono text-[11px] lowercase truncate transition-colors hover:bg-panel',
                      key === value ? 'text-ink' : 'text-ink-faint',
                    )}
                  >
                    <span className="w-3">
                      {key === value && <Check className="w-3 h-3" />}
                    </span>
                    <span className="truncate">{key}</span>
                  </button>
                ))}
                {trimmed && filtered.length === 0 && (
                  <div className="px-2 py-1 font-mono text-[11px] text-ink-ghost lowercase">
                    press enter to group by “{trimmed}”
                  </div>
                )}
              </div>
            </div>
          </div>,
          document.body,
        )}
    </>
  )
}
