import { type Ref, useId } from 'react'
import { AutocompactTokenInput } from '@/components/chat/AutocompactTokenInput'
import { RadioGroup, RadioGroupItem } from '@/components/shadcn/radio-group'
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
 * The project dialog's "Auto-compact" (§3.3.2): use the global setting
 * ("at 400k" or "at the model's limit"), compact at a size (a field and the
 * preset chips 100k–800k), or only at the model's limit. The dialog keeps
 * the draft and sends it with `harness::projects::create|update`.
 */
export function ProjectAutocompactSection({
  mode,
  text,
  onModeChange,
  onTextChange,
  global,
  error,
  firstRef,
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
  /** The first radio, for the dialog opened on this section. */
  firstRef?: Ref<HTMLElement>
  fieldRef?: Ref<HTMLInputElement>
}) {
  const ids = useId()
  const parsed = parseTokens(text)
  const typed = parsed.ok ? parsed.tokens : null
  return (
    <section
      data-project-autocompact=""
      aria-labelledby={`${ids}-label`}
      className="flex flex-col gap-2"
    >
      <span id={`${ids}-label`} className="text-body font-medium text-ink">
        {SECTION_LABEL}
      </span>
      <RadioGroup
        aria-labelledby={`${ids}-label`}
        value={mode}
        onValueChange={(next) => onModeChange(next as ProjectAutocompactMode)}
      >
        <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
          <RadioGroupItem
            ref={firstRef}
            id={`${ids}-inherit`}
            value="inherit"
          />
          <label htmlFor={`${ids}-inherit`}>{USE_GLOBAL}</label>
          <span className="text-xs text-ink-faint">{globalNow(global)}</span>
        </div>
        <div className="flex flex-col gap-2">
          <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
            <RadioGroupItem id={`${ids}-tokens`} value="tokens" />
            <label htmlFor={`${ids}-tokens`}>{VALUE_LEAD}</label>
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
        </div>
        <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
          <RadioGroupItem id={`${ids}-off`} value="off" />
          <label htmlFor={`${ids}-off`}>{ONLY_MODEL_LIMIT}</label>
        </div>
      </RadioGroup>
      {error ? (
        <p id={`${ids}-error`} role="alert" className="text-xs text-alert">
          {error}
        </p>
      ) : null}
      <p id={`${ids}-hint`} className="pl-6 text-xs text-ink-faint">
        {PROJECT_HINT}
      </p>
    </section>
  )
}
