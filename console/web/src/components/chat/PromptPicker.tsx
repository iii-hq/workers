import { useEffect, useRef, useState } from 'react'
import { Select } from '@/components/ui/Select'
import { getPrompt, listPrompts, type PromptRow } from '@/lib/prompts'

/**
 * In-chat system prompt picker — "which system prompt is this chat
 * running". `default` defers to the harness identity chain (router
 * override, provider-declared, embedded fallback). Picking a `kind:
 * system` entry from the prompt store overrides the system prompt for
 * every following send in THIS conversation, so a five-line special-
 * purpose prompt (blog writing, scraping) fully replaces the general
 * agent prompt. Lives next to the composer as one compact dropdown;
 * mid-conversation switches apply from the next turn.
 */

export interface SessionPrompt {
  name: string
  body: string
}

interface PromptPickerProps {
  /** Prompt for THIS conversation; null = the harness default chain. */
  value: SessionPrompt | null
  onChange: (next: SessionPrompt | null) => void
  disabled?: boolean
}

const DEFAULT = '(default)'

export function PromptPicker({ value, onChange, disabled }: PromptPickerProps) {
  const [prompts, setPrompts] = useState<PromptRow[]>([])
  // Monotonic token so a slow getPrompt from an earlier selection can't
  // land after a newer one and apply a stale prompt.
  const selectionRef = useRef(0)

  useEffect(() => {
    let cancelled = false
    void listPrompts()
      .then((next) => {
        if (!cancelled) {
          setPrompts(next.filter((p) => p.kind === 'system'))
        }
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const known = prompts.some((p) => p.name === value?.name)
  const options = [
    {
      value: DEFAULT,
      label: 'prompt: default',
      title:
        'the harness identity chain (router override, provider prompt, fallback)',
    },
    ...prompts.map((p) => ({
      value: p.name,
      label: `prompt: ${p.name}`,
      title: p.description,
    })),
    ...(value && !known
      ? [
          {
            value: value.name,
            label: `prompt: ${value.name}`,
            title: 'no longer in the prompt store; still applied to this chat',
          },
        ]
      : []),
  ]

  return (
    <Select<string>
      value={value?.name ?? DEFAULT}
      onChange={(next) => {
        // Every selection (including DEFAULT) supersedes any in-flight read.
        const token = ++selectionRef.current
        if (next === DEFAULT) {
          onChange(null)
          return
        }
        void getPrompt(next)
          .then((detail) => {
            if (token !== selectionRef.current) return
            if (detail) onChange({ name: detail.name, body: detail.body })
          })
          .catch(() => {
            // Failed read: leave the current selection untouched.
          })
      }}
      disabled={disabled}
      aria-label="system prompt"
      className="min-w-[104px] max-w-[180px]"
      options={options}
    />
  )
}
