import type { Ref } from 'react'
import { useId } from 'react'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from '@/components/shadcn/select'
import { FormSelectTrigger } from '@/components/ui/FormDialog'
import {
  COMPACT_AT_CHOICE,
  DETAILS_HINT,
  ONLY_MODEL_LIMIT,
  SECTION_LABEL,
  VALUE_LEAD,
  VALUE_TRAIL,
} from '@/lib/autocompact'
import { AutocompactTokenInput } from './AutocompactTokenInput'

/** The row's three choices: the inherited value, a size, the model's limit. */
export type AutocompactChoice = 'inherit' | 'tokens' | 'off'

/**
 * Conversation details' "Auto-compact" row (§3.3.4): the project's value
 * ("Project's (160k)", or "Global (400k)" without a project), "Compact
 * at…" with a token field, or "Only at the model's limit". It writes
 * `metadata.autocompact`. A side chat reads a line only (`lockedText`): it
 * compacts as its conversation does (the harness reads the parent's value).
 */
export function ConversationAutocompactRow({
  choice,
  text,
  onChoiceChange,
  onTextChange,
  inheritLabel,
  error,
  lockedText,
  triggerRef,
  fieldRef,
}: {
  choice: AutocompactChoice
  /** The token field's text. */
  text: string
  onChoiceChange: (next: AutocompactChoice) => void
  onTextChange: (next: string) => void
  /** "Project's (160k)" or "Global (400k)". */
  inheritLabel: string
  error?: string | null
  /** A side chat: this line, and no select. */
  lockedText?: string | null
  triggerRef?: Ref<HTMLButtonElement>
  fieldRef?: Ref<HTMLInputElement>
}) {
  const ids = useId()
  const readOnly = lockedText != null
  const items: { value: AutocompactChoice; label: string }[] = [
    { value: 'inherit', label: inheritLabel },
    { value: 'tokens', label: COMPACT_AT_CHOICE },
    { value: 'off', label: ONLY_MODEL_LIMIT },
  ]
  return (
    <div data-conversation-autocompact="" className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-3">
        <span
          id={`${ids}-label`}
          className="flex-1 text-body font-medium text-foreground"
        >
          {SECTION_LABEL}
        </span>
        {readOnly ? null : (
          <Select
            value={choice}
            onValueChange={(next) => onChoiceChange(next as AutocompactChoice)}
            items={items}
          >
            <FormSelectTrigger
              ref={triggerRef}
              aria-labelledby={`${ids}-label`}
              className="h-9 min-w-44"
            >
              <SelectValue />
            </FormSelectTrigger>
            <SelectContent align="end">
              {items.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>
      {readOnly ? (
        <p data-autocompact-locked="" className="text-xs text-ink-faint">
          {lockedText}
        </p>
      ) : null}
      {!readOnly && choice === 'tokens' ? (
        <div className="flex flex-wrap items-center justify-end gap-2 text-sm text-ink">
          <label htmlFor={`${ids}-tokens`}>{VALUE_LEAD}</label>
          <AutocompactTokenInput
            ref={fieldRef}
            id={`${ids}-tokens`}
            name="conversation-autocompact-tokens"
            value={text}
            onChange={onTextChange}
            invalid={Boolean(error)}
            aria-describedby={error ? `${ids}-error` : `${ids}-hint`}
          />
          <span>{VALUE_TRAIL}</span>
        </div>
      ) : null}
      {!readOnly && error ? (
        <p id={`${ids}-error`} role="alert" className="text-xs text-alert">
          {error}
        </p>
      ) : null}
      {!readOnly && choice === 'tokens' && !error ? (
        <p id={`${ids}-hint`} className="text-xs text-ink-faint">
          {DETAILS_HINT}
        </p>
      ) : null}
    </div>
  )
}
