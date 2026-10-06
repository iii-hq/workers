import { ExternalLink } from 'lucide-react'
import type { ReactNode } from 'react'
import { Input } from '@/components/ui/Input'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import {
  DEFAULT_ENV_FILE,
  defaultKeyInput,
  envRef,
  envSource,
  type KeyDetection,
  type KeyInput,
  type KeyStore,
  keyStore,
  preferredSource,
  secretRef,
  sourceLabel,
} from '@/lib/secrets'

const STORE_OPTIONS: { value: KeyStore; label: string }[] = [
  { value: 'vault', label: 'Encrypted' },
  { value: 'env', label: 'Environment variable' },
]

/**
 * How a key gets to the secrets worker. Encrypted (the default): the one
 * already stored under its name, the one found on this machine (imported by
 * the secrets worker, never shown here), or a pasted one. As an environment
 * variable, when the consumer reads `env://` references: the variable as it
 * already is in the env file the secrets worker is configured with (`.env`
 * by default), a copy of the one in the shell profile, or a pasted one
 * written to that file. Shared by the setup wizard and
 * every key field, so choosing a key reads the same everywhere.
 */
export function KeyChoice({
  name,
  detection,
  value,
  onChange,
  keysUrl,
  disabled,
  stores = ['vault'],
  envFile = DEFAULT_ENV_FILE,
}: {
  /** The secret name; also the variable the key was looked up under. */
  name: string
  detection: KeyDetection | null
  value: KeyInput | undefined
  onChange: (next: KeyInput) => void
  keysUrl?: string
  disabled?: boolean
  /** Where the consumer can read the key from; `env` adds the choice. */
  stores?: readonly KeyStore[]
  /** The secrets worker's env file, by name (`.env.staging`). */
  envFile?: string
}) {
  const store = value ? keyStore(value) : 'vault'
  const pasted = value?.mode === 'paste' ? value.value : ''
  const group = `key-${name}`
  const paste = (
    <PasteInput
      name={name}
      value={pasted}
      onChange={(next) => onChange({ mode: 'paste', value: next, store })}
      keysUrl={keysUrl}
    />
  )
  return (
    <fieldset className="flex flex-col gap-1.5" disabled={disabled}>
      <legend className="sr-only">{`Where ${name} comes from`}</legend>
      {stores.includes('env') && stores.includes('vault') ? (
        <SegmentedControl
          variant="radio"
          aria-label={`Where to keep ${name}`}
          value={store}
          onChange={(next) => onChange(defaultKeyInput(detection, next))}
          options={STORE_OPTIONS}
          className="mb-1 w-fit"
        />
      ) : null}
      {store === 'env' ? (
        <EnvOptions
          name={name}
          envFile={envFile}
          group={group}
          detection={detection}
          value={value}
          onChange={onChange}
          paste={paste}
        />
      ) : (
        <VaultOptions
          group={group}
          detection={detection}
          value={value}
          onChange={onChange}
          paste={paste}
        />
      )}
    </fieldset>
  )
}

function VaultOptions({
  group,
  detection,
  value,
  onChange,
  paste,
}: {
  group: string
  detection: KeyDetection | null
  value: KeyInput | undefined
  onChange: (next: KeyInput) => void
  paste: ReactNode
}) {
  const found = preferredSource(detection)
  const stored = detection?.stored === true
  const mode = value?.mode ?? (stored ? 'stored' : found ? 'import' : 'paste')
  const pasted = value?.mode === 'paste' ? value.value : ''
  return (
    <>
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
      {mode === 'paste' ? paste : null}
    </>
  )
}

function EnvOptions({
  name,
  envFile,
  group,
  detection,
  value,
  onChange,
  paste,
}: {
  name: string
  envFile: string
  group: string
  detection: KeyDetection | null
  value: KeyInput | undefined
  onChange: (next: KeyInput) => void
  paste: ReactNode
}) {
  const current = envSource(detection)
  const shell = current
    ? null
    : (detection?.sources.find((source) => source.kind === 'login_shell') ??
      null)
  const mode = value?.mode ?? (current ? 'env' : shell ? 'import' : 'paste')
  const pasted = value?.mode === 'paste' ? value.value : ''
  return (
    <>
      {current ? (
        <KeyOption
          name={group}
          checked={mode === 'env'}
          onSelect={() => onChange({ mode: 'env' })}
          label={`Use ${name} from ${sourceLabel(current)}`}
          meta={current.hint}
        />
      ) : null}
      {shell ? (
        <KeyOption
          name={group}
          checked={mode === 'import'}
          onSelect={() =>
            onChange({ mode: 'import', source: shell.kind, store: 'env' })
          }
          label={`Copy the key from ${sourceLabel(shell)} into ${envFile}`}
          meta={shell.hint}
        />
      ) : null}
      {current || shell ? (
        <KeyOption
          name={group}
          checked={mode === 'paste'}
          onSelect={() =>
            onChange({ mode: 'paste', value: pasted, store: 'env' })
          }
          label={
            current
              ? `Paste a different key into ${envFile}`
              : `Paste a key into ${envFile}`
          }
        />
      ) : null}
      {mode === 'paste' ? paste : null}
    </>
  )
}

/** Where the chosen key ends up, and the reference configuration gets. */
export function KeyDestination({
  name,
  input,
  className,
  envFile = DEFAULT_ENV_FILE,
}: {
  name: string
  input: KeyInput
  className?: string
  /** The secrets worker's env file, by name. */
  envFile?: string
}) {
  return (
    <p
      className={
        className ?? 'font-sans text-[12px] leading-relaxed text-ink-faint'
      }
    >
      {keyStore(input) === 'env' ? (
        <>
          The secrets worker reads <span className="font-mono">{name}</span>{' '}
          from this project's <span className="font-mono">{envFile}</span> each
          time it is used. Configuration gets{' '}
          <span className="font-mono text-ink">{envRef(name)}</span>.
        </>
      ) : (
        <>
          Stored encrypted by the secrets worker. Configuration gets{' '}
          <span className="font-mono text-ink">{secretRef(name)}</span>, so the
          key never lands in a file you commit.
        </>
      )}
    </p>
  )
}

function PasteInput({
  name,
  value,
  onChange,
  keysUrl,
}: {
  name: string
  value: string
  onChange: (next: string) => void
  keysUrl?: string
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Input
        type="password"
        autoComplete="off"
        spellCheck={false}
        aria-label={name}
        placeholder={`Paste your ${name}`}
        value={value}
        onChange={onChange}
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
