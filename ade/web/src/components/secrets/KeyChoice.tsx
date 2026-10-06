import { ExternalLink } from 'lucide-react'
import { Input } from '@/components/ui/Input'
import {
  type KeyDetection,
  type KeyInput,
  preferredSource,
  sourceLabel,
} from '@/lib/secrets'

/**
 * How a key gets into the secrets store: the one already stored under its
 * name, the one found on this machine (imported by the secrets worker, never
 * shown here), or a pasted one. Shared by the setup wizard and every key
 * field, so choosing a key reads the same everywhere.
 */
export function KeyChoice({
  name,
  detection,
  value,
  onChange,
  keysUrl,
  disabled,
}: {
  /** The secret name; also the variable the key was looked up under. */
  name: string
  detection: KeyDetection | null
  value: KeyInput | undefined
  onChange: (next: KeyInput) => void
  keysUrl?: string
  disabled?: boolean
}) {
  const found = preferredSource(detection)
  const stored = detection?.stored === true
  const mode = value?.mode ?? (stored ? 'stored' : found ? 'import' : 'paste')
  const pasted = value?.mode === 'paste' ? value.value : ''
  const group = `key-${name}`
  return (
    <fieldset className="flex flex-col gap-1.5" disabled={disabled}>
      <legend className="sr-only">{`Where ${name} comes from`}</legend>
      {stored ? (
        <KeyOption
          name={group}
          checked={mode === 'stored'}
          onSelect={() => onChange({ mode: 'stored' })}
          label="Use the key already in the secrets store"
          meta={detection?.stored_hint ?? undefined}
        />
      ) : null}
      {found ? (
        <KeyOption
          name={group}
          checked={mode === 'import'}
          onSelect={() => onChange({ mode: 'import', source: found.kind })}
          label={`Use the key from ${sourceLabel(found)}`}
          meta={found.hint}
        />
      ) : null}
      {stored || found ? (
        <KeyOption
          name={group}
          checked={mode === 'paste'}
          onSelect={() => onChange({ mode: 'paste', value: pasted })}
          label="Paste a different key"
        />
      ) : null}
      {mode === 'paste' ? (
        <div className="flex flex-col gap-1.5">
          <Input
            type="password"
            autoComplete="off"
            spellCheck={false}
            aria-label={name}
            placeholder={`Paste your ${name}`}
            value={pasted}
            onChange={(next) => onChange({ mode: 'paste', value: next })}
            className="font-mono"
          />
          {keysUrl ? (
            <a
              href={keysUrl}
              target="_blank"
              rel="noreferrer"
              className="inline-flex w-fit items-center gap-1 rounded-sm font-sans text-[12px] text-ink-faint hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
            >
              Create a key
              <ExternalLink className="size-4" aria-hidden />
            </a>
          ) : null}
        </div>
      ) : null}
    </fieldset>
  )
}

function KeyOption({
  name,
  checked,
  onSelect,
  label,
  meta,
}: {
  name: string
  checked: boolean
  onSelect: () => void
  label: string
  meta?: string
}) {
  return (
    <label className="flex min-h-9 cursor-pointer items-center gap-2.5 rounded-sm px-2 hover:bg-surface-hover">
      <input
        type="radio"
        name={name}
        checked={checked}
        onChange={onSelect}
        className="size-4 shrink-0 accent-[var(--color-accent)]"
      />
      <span className="min-w-0 flex-1 font-sans text-[13px] text-ink">
        {label}
      </span>
      {meta ? (
        <span className="shrink-0 font-mono text-[11px] text-ink-faint">
          {meta}
        </span>
      ) : null}
    </label>
  )
}
