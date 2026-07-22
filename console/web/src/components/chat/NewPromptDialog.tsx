import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Input } from '@/components/ui/Input'
import { Select } from '@/components/ui/Select'
import {
  createPrompt,
  PROMPT_NAME_RE,
  type PromptStrategy,
} from '@/lib/prompts'

/**
 * Minimal create-prompt dialog for the composer's system-prompt picker.
 * Writes via `directory::prompts::save` (create-only; server errors — e.g.
 * name collisions — render inline). Plain textarea by design: prompt
 * bodies are markdown, not `${VAR}` config templates.
 */

interface NewPromptDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Fired after a successful save with the created prompt's name. */
  onCreated: (name: string) => void
}

export function NewPromptDialog({
  open,
  onOpenChange,
  onCreated,
}: NewPromptDialogProps) {
  const [name, setName] = useState('')
  const [body, setBody] = useState('')
  const [strategy, setStrategy] = useState<PromptStrategy>('enrich')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const nameValid = PROMPT_NAME_RE.test(name)
  const canSave = nameValid && body.trim().length > 0 && !saving

  const reset = () => {
    setName('')
    setBody('')
    setStrategy('enrich')
    setError(null)
    setSaving(false)
  }

  const save = async () => {
    if (!canSave) return
    setSaving(true)
    setError(null)
    try {
      await createPrompt({ name, body, strategy })
      const created = name
      reset()
      onOpenChange(false)
      onCreated(created)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setSaving(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset()
        onOpenChange(next)
      }}
    >
      <DialogContent className="max-w-lg">
        <DialogTitle className="text-[14px] lowercase">
          new system prompt
        </DialogTitle>
        <DialogDescription className="text-[12px] text-ink-faint">
          saved as a markdown file in the prompt library; usable as this chat's
          system prompt right away.
        </DialogDescription>
        <div className="flex flex-col gap-3 pt-2">
          <div className="flex flex-col gap-1">
            <label
              htmlFor="new-prompt-name"
              className="text-[11px] lowercase text-ink-faint"
            >
              name
            </label>
            <Input
              id="new-prompt-name"
              value={name}
              onChange={setName}
              placeholder="code-reviewer"
            />
            {name.length > 0 && !nameValid ? (
              <p className="text-[11px] text-warn">
                lowercase letters, digits, - and _ only (max 64)
              </p>
            ) : null}
          </div>
          <div className="flex flex-col gap-1">
            <label
              htmlFor="new-prompt-strategy"
              className="text-[11px] lowercase text-ink-faint"
            >
              strategy
            </label>
            <Select<PromptStrategy>
              value={strategy}
              onChange={setStrategy}
              aria-label="prompt strategy"
              options={[
                {
                  value: 'enrich',
                  label: 'enrich — add to the built-in prompt',
                  title:
                    'appends this prompt after the built-in identity prompt (safe default)',
                },
                {
                  value: 'override',
                  label: 'override — replace it',
                  title: 'this prompt becomes the entire system prompt',
                },
              ]}
            />
          </div>
          <div className="flex flex-col gap-1">
            <label
              htmlFor="new-prompt-body"
              className="text-[11px] lowercase text-ink-faint"
            >
              prompt
            </label>
            <textarea
              id="new-prompt-body"
              value={body}
              onChange={(e) => setBody(e.target.value)}
              rows={8}
              spellCheck={false}
              placeholder="you are a…"
              className="w-full resize-y border border-rule bg-bg p-3 font-mono text-[13px] leading-relaxed text-ink outline-none transition-colors placeholder:text-ink-ghost focus:border-ink"
            />
          </div>
          {error ? (
            <p className="break-words text-[11px] text-warn">{error}</p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button
              variant="ghost"
              size="sm"
              type="button"
              onClick={() => onOpenChange(false)}
            >
              cancel
            </Button>
            <Button
              size="sm"
              type="button"
              disabled={!canSave}
              onClick={() => void save()}
            >
              {saving ? 'saving…' : 'save'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
