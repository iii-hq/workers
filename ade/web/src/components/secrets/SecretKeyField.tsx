import { formatRelative } from '@iii-dev/console-ui/format'
import { CircleAlert, KeyRound, LoaderCircle } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Chip } from '@/components/ui/Chip'
import { Skeleton } from '@/components/ui/Skeleton'
import { addWorkersWithProgress, readableError } from '@/lib/onboarding/api'
import {
  defaultKeyInput,
  detectKeys,
  engineVarName,
  envFileName,
  envRefName,
  getSecret,
  getSecretsStatus,
  type KeyDetection,
  type KeyInput,
  type KeyStore,
  keyInputReady,
  keyReference,
  keyStore,
  SECRETS_WORKER,
  type SecretMeta,
  secretRefName,
  storeKey,
} from '@/lib/secrets'
import { cn } from '@/lib/utils'
import { KeyChoice, KeyDestination } from './KeyChoice'

/** What the worker that uses the key reports about it, when it reports. */
export interface SecretKeyStatus {
  /** The consumer resolved a credential. */
  connected?: boolean
  /** `config | env | secret | none`, as llm-router reports it. */
  source?: string
  /** Why the reference did not resolve, in the consumer's words. */
  error?: string
  /** The consumer is still checking the key it was just given. */
  checking?: boolean
  /** What the key unlocked, shown beside the status (`11 models`). */
  detail?: string
}

export interface SecretKeyFieldProps {
  /** Secret name — also the environment variable the key is looked up under. */
  name: string
  /**
   * The field's configuration value: `secret://NAME`, `env://NAME`,
   * `${VAR}`, a literal, or empty.
   */
  value: string | undefined
  /**
   * Write the field: a `secret://` or `env://` reference, or `undefined` to
   * clear it. May return a promise; the field shows its failure.
   */
  onChange: (next: string | undefined) => unknown
  /** Workers allowed to read the key (`llm-router` for provider keys). */
  consumers: readonly string[]
  label?: string
  status?: SecretKeyStatus
  /** Where to create a key. */
  keysUrl?: string
  /**
   * The consumer also resolves `env://NAME` (llm-router does): offer keeping
   * the key as an environment variable in the secrets worker's env file
   * (`.env` unless its configuration names another).
   */
  environment?: boolean
  disabled?: boolean
  className?: string
}

type Busy = 'store' | 'remove' | 'install' | null

function updatedLabel(at: string): string {
  const relative = formatRelative(at)
  return relative === 'just now'
    ? 'updated just now'
    : `updated ${relative} ago`
}

/** `secrets::get`'s `location` for the env store, in words. */
function envLocationLabel(location: string | null | undefined): string {
  return location && /[\\/]/.test(location)
    ? `this project's ${envFileName(location)}`
    : "the secrets worker's environment"
}

/**
 * One credential, kept by the secrets worker: encrypted, with only
 * `secret://NAME` in configuration, or — where the consumer reads them — as
 * an environment variable in the secrets worker's env file, with `env://NAME`. It
 * shows what is in use and offers the same choices as the setup wizard:
 * reuse a key found on this machine, keep the stored one, or paste a new
 * one; plus moving a plain-text key into the store, replacing it, and
 * removing the reference. The secrets worker comes with llm-router; if it is
 * not running, the field offers to start it again.
 */
export function SecretKeyField({
  name,
  value,
  onChange,
  consumers,
  label = 'API key',
  status,
  keysUrl,
  environment,
  disabled,
  className,
}: SecretKeyFieldProps) {
  const refName = secretRefName(value)
  const envName = envRefName(value)
  const engineVar = engineVarName(value)
  const literal = Boolean(value?.trim()) && !refName && !envName && !engineVar
  const referenced = refName ?? envName
  const currentStore: KeyStore = envName ? 'env' : 'vault'
  const storedName = referenced ?? name
  const stores: readonly KeyStore[] = environment ? ['vault', 'env'] : ['vault']

  // undefined: still loading · null: nothing kept under that name
  const [meta, setMeta] = useState<SecretMeta | null | undefined>(undefined)
  // undefined: still loading · false: the secrets worker is not running
  const [secretsUp, setSecretsUp] = useState<boolean | undefined>(undefined)
  const [detection, setDetection] = useState<KeyDetection | null>(null)
  const [envFile, setEnvFile] = useState<string>(envFileName(null))
  const [editing, setEditing] = useState(false)
  const [input, setInput] = useState<KeyInput | undefined>(undefined)
  const [busy, setBusy] = useState<Busy>(null)
  const [note, setNote] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reload, setReload] = useState(0)

  // biome-ignore lint/correctness/useExhaustiveDependencies: `reload` re-reads the store after this field changed it
  useEffect(() => {
    let cancelled = false
    setMeta(undefined)
    void (async () => {
      try {
        const [current, found, status] = await Promise.all([
          getSecret(storedName, currentStore),
          detectKeys([name]),
          environment ? getSecretsStatus().catch(() => undefined) : undefined,
        ])
        if (cancelled) return
        setSecretsUp(current !== undefined)
        setMeta(current ?? null)
        setDetection(found?.[0] ?? null)
        setEnvFile(envFileName(status?.env_file))
      } catch (cause) {
        if (cancelled) return
        setSecretsUp(true)
        setMeta(null)
        setError(readableError(cause))
      }
    })()
    return () => {
      cancelled = true
    }
  }, [name, storedName, currentStore, environment, reload])

  const choose = useCallback(() => {
    setEditing(true)
    setError(null)
    setInput(
      defaultKeyInput(detection, envName && environment ? 'env' : 'vault'),
    )
  }, [detection, envName, environment])

  const store = async (next: KeyInput) => {
    setBusy('store')
    setError(null)
    try {
      await storeKey(
        name,
        next,
        consumers,
        `${label} for ${consumers.join(', ')}, stored from the ADE`,
      )
      await onChange(keyReference(name, next))
      setEditing(false)
      setInput(undefined)
      setReload((count) => count + 1)
    } catch (cause) {
      setError(readableError(cause))
    } finally {
      setBusy(null)
    }
  }

  const remove = async () => {
    setBusy('remove')
    setError(null)
    try {
      await onChange(undefined)
    } catch (cause) {
      setError(readableError(cause))
    } finally {
      setBusy(null)
    }
  }

  const install = async () => {
    setBusy('install')
    setError(null)
    try {
      await addWorkersWithProgress([SECRETS_WORKER], ({ note: line }) =>
        setNote(line ?? null),
      )
      setNote(null)
      setReload((count) => count + 1)
    } catch (cause) {
      setError(readableError(cause))
      setNote(null)
    } finally {
      setBusy(null)
    }
  }

  const tone = status?.checking
    ? 'neutral'
    : status?.error
      ? 'warning'
      : status?.connected
        ? 'success'
        : 'neutral'
  const chip = status?.checking
    ? 'Checking…'
    : status?.error
      ? 'Needs attention'
      : status?.connected
        ? ['Connected', status.detail].filter(Boolean).join(' · ')
        : status
          ? 'Not connected'
          : null
  const locked = disabled || busy !== null

  const actions = (
    <span className="flex shrink-0 items-center gap-1">
      <Button variant="ghost" size="sm" onClick={choose} disabled={locked}>
        Replace
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => void remove()}
        disabled={locked}
      >
        {busy === 'remove' ? 'Removing…' : 'Remove'}
      </Button>
    </span>
  )

  let body: ReactNode
  if (secretsUp === undefined) {
    body = <Skeleton className="h-9 w-full" />
  } else if (!secretsUp) {
    body = (
      <div className="flex flex-col gap-2">
        <p className="font-sans text-[13px] leading-relaxed text-ink-faint">
          The <span className="font-mono">secrets</span> worker, which keeps API
          keys for llm-router, is not running.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="pill"
            size="sm"
            onClick={() => void install()}
            disabled={locked}
          >
            {busy === 'install' ? (
              <LoaderCircle className="iii-ui-spin" aria-hidden />
            ) : null}
            {busy === 'install'
              ? 'Starting the secrets worker…'
              : 'Start the secrets worker'}
          </Button>
          <span className="font-mono text-[11px] text-ink-ghost">
            compose::add {SECRETS_WORKER}
          </span>
        </div>
        {note ? (
          <p role="status" className="font-sans text-[12px] text-ink-faint">
            {note}
          </p>
        ) : null}
      </div>
    )
  } else if (editing || (!value?.trim() && !referenced)) {
    const pending = input ?? defaultKeyInput(detection)
    body = (
      <div className="flex flex-col gap-2">
        {!value?.trim() && status?.source === 'env' ? (
          <p className="font-sans text-[12px] text-ink-faint">
            In use now: <span className="font-mono">{name}</span> from the
            worker's environment. Choose where to keep it to manage it here.
          </p>
        ) : null}
        <KeyChoice
          name={name}
          detection={detection}
          value={pending}
          onChange={setInput}
          keysUrl={keysUrl}
          disabled={locked}
          stores={stores}
          envFile={envFile}
        />
        <KeyDestination name={name} input={pending} envFile={envFile} />
        <div className="flex items-center justify-end gap-1">
          {editing && (value?.trim() || referenced) ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setEditing(false)}
              disabled={locked}
            >
              Cancel
            </Button>
          ) : null}
          <Button
            variant="pill"
            size="sm"
            onClick={() => void store(pending)}
            disabled={locked || !keyInputReady(pending)}
          >
            {busy === 'store'
              ? 'Saving…'
              : pending.mode === 'env'
                ? 'Use variable'
                : keyStore(pending) === 'env'
                  ? `Save to ${envFile}`
                  : 'Save key'}
          </Button>
        </div>
      </div>
    )
  } else if (refName) {
    body = (
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="flex min-w-0 flex-col">
          <span className="truncate font-mono text-[12px] text-ink">
            {value?.trim()}
          </span>
          <span className="font-sans text-[12px] text-ink-faint">
            {meta === undefined
              ? 'Reading the secrets store…'
              : meta
                ? [
                    'Encrypted',
                    meta.hint,
                    meta.updated_at ? updatedLabel(meta.updated_at) : null,
                  ]
                    .filter(Boolean)
                    .join(' · ')
                : `Not in the secrets store`}
          </span>
        </span>
        {actions}
      </div>
    )
  } else if (envName) {
    body = (
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="flex min-w-0 flex-col">
          <span className="truncate font-mono text-[12px] text-ink">
            {value?.trim()}
          </span>
          <span className="font-sans text-[12px] text-ink-faint">
            {meta === undefined
              ? 'Reading the secrets worker…'
              : meta === null
                ? `Not shared with ${consumers.join(', ')} yet`
                : meta.hint
                  ? `Environment variable · ${meta.hint} · from ${envLocationLabel(meta.location)}`
                  : `Not set in this project's ${envFile} or the secrets worker's environment`}
          </span>
        </span>
        {actions}
      </div>
    )
  } else {
    // A plain-text key or an engine variable: what it is, and the move.
    body = (
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="min-w-0 font-sans text-[12px] text-ink-faint">
          {literal ? (
            <>
              Saved as plain text in configuration, which is meant to be
              committed.
            </>
          ) : (
            <>
              Expanded by the engine from its{' '}
              <span className="font-mono text-ink">{engineVar}</span> variable.
            </>
          )}
        </span>
        <Button
          variant="pill"
          size="sm"
          onClick={() =>
            literal && value
              ? void store({ mode: 'paste', value: value.trim() })
              : choose()
          }
          disabled={locked}
        >
          {busy === 'store'
            ? 'Moving…'
            : literal
              ? 'Move to secrets store'
              : 'Choose where to keep it'}
        </Button>
      </div>
    )
  }

  return (
    <section
      aria-label={label}
      className={cn(
        'flex flex-col gap-2.5 rounded-md bg-surface p-3',
        className,
      )}
    >
      <header className="flex items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-2">
          <KeyRound className="size-4 shrink-0 text-ink-faint" aria-hidden />
          <span className="font-sans text-[13px] font-medium text-ink">
            {label}
          </span>
        </span>
        {chip ? <Chip tone={tone}>{chip}</Chip> : null}
      </header>
      {body}
      {status?.error && !status.checking ? (
        <p className="flex items-start gap-1.5 font-sans text-[12px] text-warn-strong">
          <CircleAlert className="mt-px size-4 shrink-0" aria-hidden />
          <span>{status.error}</span>
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="font-sans text-[12px] text-alert-strong">
          {error}
        </p>
      ) : null}
    </section>
  )
}
