import { type ReactNode, type Ref, useId } from 'react'
import { AutocompactTokenInput } from '@/components/chat/AutocompactTokenInput'
import {
  AUTOCOMPACT_PRESETS,
  formatTokens,
  type GlobalAutocompact,
  globalNow,
  ONLY_MODEL_LIMIT,
  PROJECT_HINT,
  parseTokens,
  SECTION_LABEL,
  USE_GLOBAL,
  VALUE_LEAD,
  VALUE_TRAIL,
} from '@/lib/autocompact'
import { cn } from '@/lib/utils'

/** The section's three choices (§3.3.2). */
export type ProjectAutocompactMode = 'inherit' | 'tokens' | 'off'

/**
 * One of the three choices: a native radio, grouped with the others by
 * `name`, drawn as the design system's radio (a filled ring with a dot).
 */
function Choice({
  id,
  name,
  value,
  mode,
  onModeChange,
  choiceRef,
  children,
}: {
  id: string
  name: string
  value: ProjectAutocompactMode
  mode: ProjectAutocompactMode
  onModeChange: (next: ProjectAutocompactMode) => void
  choiceRef?: Ref<HTMLInputElement>
  children: ReactNode
}) {
  return (
    <>
      <input
        ref={mode === value ? choiceRef : undefined}
        type="radio"
        id={id}
        name={name}
        value={value}
        checked={mode === value}
        onChange={() => onModeChange(value)}
        className="size-4 shrink-0 appearance-none rounded-full border border-muted-foreground/60 outline-none transition-colors hover:border-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 checked:border-4 checked:border-primary checked:bg-primary-foreground dark:bg-input/30 dark:checked:bg-primary-foreground"
      />
      <label htmlFor={id}>{children}</label>
    </>
  )
}

/**
 * The project dialog's "Auto-compact" (§3.3.2): use the global setting
 * ("at 400k" or "at the model's limit"), compact at a size (a field and the
 * preset chips 100k–800k), or only at the model's limit. The dialog keeps
 * the draft and sends it with `harness::projects::create|update`.
 *
 * The three choices are native radios sharing one `name`, so assistive
 * tech reads a group of exactly three; the size field and the presets are
 * their siblings inside the section's fieldset, never inside a radio group.
 */
export function ProjectAutocompactSection({
  mode,
  text,
  onModeChange,
  onTextChange,
  global,
  error,
  choiceRef,
  fieldRef,
}: {
  mode: ProjectAutocompactMode
  /** The size field's text: `160k`, `400000`, `1.2M`. */
  text: string
  onModeChange: (next: ProjectAutocompactMode) => void
  onTextChange: (next: string) => void
  /** The global setting, for "Use the global setting · at 400k". */
  global: GlobalAutocompact
  error?: string
  /** The checked choice, for the dialog opened on this section. */
  choiceRef?: Ref<HTMLInputElement>
  fieldRef?: Ref<HTMLInputElement>
}) {
  const ids = useId()
  const name = `${ids}-mode`
  const parsed = parseTokens(text)
  const typed = parsed.ok ? parsed.tokens : null
  const choice = { name, mode, onModeChange, choiceRef }
  return (
    <fieldset
      data-project-autocompact=""
      aria-describedby={`${ids}-hint`}
      className="flex min-w-0 flex-col gap-2"
    >
      <legend className="mb-2 text-body font-medium text-ink">
        {SECTION_LABEL}
      </legend>
      <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
        <Choice id={`${ids}-inherit`} value="inherit" {...choice}>
          {USE_GLOBAL}
        </Choice>
        <span className="text-xs text-ink-faint">{globalNow(global)}</span>
      </div>
      <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
        <Choice id={`${ids}-tokens`} value="tokens" {...choice}>
          {VALUE_LEAD}
        </Choice>
        <AutocompactTokenInput
          ref={fieldRef}
          aria-label={`${VALUE_LEAD} (${VALUE_TRAIL})`}
          name="project-autocompact-tokens"
          value={text}
          onChange={(next) => {
            onTextChange(next)
            if (mode !== 'tokens') onModeChange('tokens')
          }}
          onFocus={() => {
            if (mode !== 'tokens') onModeChange('tokens')
          }}
          invalid={Boolean(error)}
          aria-describedby={error ? `${ids}-error` : `${ids}-hint`}
          className="h-8"
        />
        <span>{VALUE_TRAIL}</span>
      </div>
      <fieldset
        aria-label="Sizes"
        className="flex flex-wrap items-center gap-1.5 pl-6"
      >
        {AUTOCOMPACT_PRESETS.map((preset) => {
          const selected = mode === 'tokens' && typed === preset
          return (
            <button
              key={preset}
              type="button"
              data-autocompact-preset={preset}
              aria-pressed={selected}
              onClick={() => {
                onTextChange(formatTokens(preset))
                onModeChange('tokens')
              }}
              className={cn(
                'inline-flex h-7 items-center rounded-md px-2 font-mono text-xs tabular-nums focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 pointer-coarse:h-11',
                selected
                  ? 'bg-surface-selected text-ink'
                  : 'bg-surface text-ink-faint hover:bg-surface-hover hover:text-ink',
              )}
            >
              {formatTokens(preset)}
            </button>
          )
        })}
      </fieldset>
      <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
        <Choice id={`${ids}-off`} value="off" {...choice}>
          {ONLY_MODEL_LIMIT}
        </Choice>
      </div>
      {error ? (
        <p id={`${ids}-error`} role="alert" className="text-xs text-alert">
          {error}
        </p>
      ) : null}
      <p id={`${ids}-hint`} className="pl-6 text-xs text-ink-faint">
        {PROJECT_HINT}
      </p>
    </fieldset>
  )
}
