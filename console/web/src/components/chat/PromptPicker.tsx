import { useEffect, useState } from 'react'
import { Select } from '@/components/ui/Select'
import { listPrompts, type PromptEntry } from '@/lib/prompts'
import { NewPromptDialog } from './NewPromptDialog'

/**
 * In-chat system-prompt picker — chooses a named prompt from the
 * iii-directory prompt library (`directory::prompts::*`). The selected
 * name persists in session metadata (`system_prompt_name`) and the send
 * path resolves it to a body + strategy on EVERY send, so on-disk edits
 * apply on the next turn. `none` = the built-in identity prompt only.
 */

interface PromptPickerProps {
  /** Prompt for THIS conversation; null = built-in prompt only. */
  value: string | null
  onChange: (next: string | null) => void
  disabled?: boolean
}

// Parens are illegal in prompt names (server-side validate_name), so these
// sentinels can never collide with a real prompt.
const NONE = '(none)'
const NEW = '(new)'

export function PromptPicker({ value, onChange, disabled }: PromptPickerProps) {
  const [prompts, setPrompts] = useState<PromptEntry[]>([])
  const [dialogOpen, setDialogOpen] = useState(false)

  useEffect(() => {
    let cancelled = false
    void listPrompts()
      .then((next) => {
        if (!cancelled) setPrompts(next)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const known = prompts.some((p) => p.name === value)
  const options = [
    {
      value: NONE,
      label: 'prompt: none',
      title: 'built-in identity prompt only',
    },
    ...prompts.map((p) => ({
      value: p.name,
      label: `prompt: ${p.name}`,
      title: `${p.description || p.name} · ${
        p.strategy === 'override'
          ? 'replaces the built-in prompt'
          : 'adds to the built-in prompt'
      }`,
    })),
    // A prompt set elsewhere (CLI, another client) that we haven't listed.
    ...(value && !known
      ? [
          {
            value,
            label: `prompt: ${value}`,
            title: 'set outside this console',
          },
        ]
      : []),
    { value: NEW, label: 'new prompt…', title: 'create a new prompt file' },
  ]

  return (
    <>
      <Select<string>
        value={value ?? NONE}
        onChange={(next) => {
          if (next === NEW) {
            // Controlled value stays put; the dialog owns what happens next.
            setDialogOpen(true)
            return
          }
          onChange(next === NONE ? null : next)
        }}
        disabled={disabled}
        aria-label="system prompt"
        className="min-w-[96px] max-w-[160px]"
        options={options}
      />
      <NewPromptDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onCreated={(name) => {
          onChange(name)
          // Re-list from the worker (live disk read) so the new entry gets
          // its real description/strategy; the unknown-selected row covers
          // the gap until it lands.
          void listPrompts()
            .then(setPrompts)
            .catch(() => {})
        }}
      />
    </>
  )
}
