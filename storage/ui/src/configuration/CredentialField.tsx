/**
 * A bucket credential — a secret access key, a session token, a
 * service-account key, a queue API token, or an identifier (access key ID,
 * account ID) the user may keep in the secrets store too.
 *
 * A new value is not written into configuration: "Save to secrets" stores
 * it in the secrets worker at once under the bucket's derived name and the
 * field becomes `secret://NAME`. What the field shows afterwards is the
 * reference and whether the store has it, never the value nor the store's
 * masked hint. A plain-text value that predates this (or was typed into
 * YAML by hand) is flagged, with a one-click move into the store;
 * `env://NAME` and `${VAR}` are shown as such. When the secrets worker is
 * not running — or the storage worker predates references — the field says
 * so and, only on an explicit choice, keeps the value inline.
 */

import { Button, Chip, type Host, Input, type JsonValue } from '@iii-dev/console-ui'
import { Upload } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { errorMessage } from '../lib/errors'
import {
  envRefName,
  envStatusText,
  getSecret,
  type SecretMeta,
  secretRefName,
  secretStatusText,
  storeSecret,
} from '../lib/secrets'
import { checkServiceAccountJson, valueKind } from './bucket'
import { Field } from './fields'

export interface CredentialFieldProps {
  host: Host
  id: string
  /** Configuration path, for deep links (`buckets/photos/secret_access_key`). */
  path: string
  label: string
  hint?: ReactNode
  error?: string
  value: JsonValue | undefined
  /** Where a new value is stored — derived from the bucket name, already unique. */
  target: string
  /** The bucket name, for the secret's description. */
  bucket: string
  /**
   * `secret`: masked input, a plain-text value is a warning. `identifier`:
   * a visible value that may stay in configuration (`preferText` makes that
   * the default action). `json`: a pasted or uploaded service-account key.
   */
  mode?: 'secret' | 'identifier' | 'json'
  preferText?: boolean
  /** `false` when the storage worker predates `secret://` references. */
  referencesSupported?: boolean | null
  placeholder?: string
  onChange: (next: string | undefined) => void
}

type Busy = 'store' | null

// biome-ignore lint/suspicious/noTemplateCurlyInString: the configuration's own template syntax, shown as text
const TEMPLATE_SYNTAX = '${VAR}'

export function CredentialField(props: CredentialFieldProps) {
  return (
    <Field label={<label htmlFor={props.id}>{props.label}</label>} hint={props.hint} error={props.error}>
      <CredentialControl {...props} />
    </Field>
  )
}

function CredentialControl({
  host,
  id,
  path,
  label,
  value,
  target,
  bucket,
  mode = 'secret',
  preferText = false,
  referencesSupported,
  placeholder,
  onChange,
}: CredentialFieldProps) {
  const kind = valueKind(value)
  const text = typeof value === 'string' ? value.trim() : ''
  const secretName = secretRefName(value)
  const envName = envRefName(value)
  const legacyWorker = referencesSupported === false
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [busy, setBusy] = useState<Busy>(null)
  const [error, setError] = useState<string | null>(null)
  // undefined: still reading · null: nothing by that name
  const [meta, setMeta] = useState<SecretMeta | null | undefined>(undefined)
  // undefined: unknown yet · false: the secrets worker is not running
  const [secretsUp, setSecretsUp] = useState<boolean | undefined>(undefined)
  const [reload, setReload] = useState(0)
  const fileRef = useRef<HTMLInputElement>(null)

  // What the row reads: the referenced secret or variable, else the name a
  // new value would take (which also tells whether the store is running).
  const probeName = secretName ?? envName ?? target
  const probeStore = envName && !secretName ? 'env' : undefined
  useEffect(() => {
    if (legacyWorker) return
    let alive = true
    setMeta(undefined)
    getSecret(host, probeName, probeStore)
      .then((found) => {
        if (!alive) return
        setSecretsUp(found !== undefined)
        setMeta(found ?? null)
      })
      .catch(() => {
        if (!alive) return
        setSecretsUp(true)
        setMeta(null)
      })
    return () => {
      alive = false
    }
  }, [host, probeName, probeStore, legacyWorker, reload])

  const canStore = !legacyWorker && secretsUp === true

  const store = async (secretValue: string) => {
    setBusy('store')
    setError(null)
    try {
      const ref = await storeSecret(host, {
        name: target,
        value: secretValue,
        description: `${label} for storage bucket “${bucket}”, stored from the console`,
      })
      onChange(ref)
      setEditing(false)
      setDraft('')
      setReload((n) => n + 1)
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setBusy(null)
    }
  }

  const keepInline = (raw: string) => {
    const kept = raw.trim()
    onChange(kept === '' ? undefined : kept)
    setEditing(false)
    setDraft('')
    setError(null)
  }

  const pickFile = async (file: File | undefined) => {
    if (!file) return
    try {
      setDraft(await file.text())
      setError(null)
    } catch (cause) {
      setError(errorMessage(cause))
    }
  }

  const locked = busy !== null
  const startEdit = (initial = '') => {
    setEditing(true)
    setDraft(initial)
    setError(null)
  }
  const replace = (
    <Button type="button" variant="ghost" size="sm" disabled={locked} onClick={() => startEdit()}>
      Replace
    </Button>
  )
  const remove = (
    <Button type="button" variant="ghost" size="sm" disabled={locked} onClick={() => onChange(undefined)}>
      Remove
    </Button>
  )
  const row = (copy: ReactNode, buttons: ReactNode) => (
    <div className="storage-cfg-secret-row">
      <span className="storage-cfg-secret-copy">{copy}</span>
      <span className="storage-cfg-secret-buttons">{buttons}</span>
    </div>
  )

  let body: ReactNode
  if (editing || kind === 'empty') {
    const draftReady = draft.trim() !== ''
    const json = mode === 'json' && draftReady ? checkServiceAccountJson(draft) : null
    const textFirst = mode === 'identifier' && preferText
    const inlineOnly = legacyWorker || secretsUp === false
    const submit = () => {
      if (!draftReady || locked) return
      if (inlineOnly || textFirst) keepInline(draft)
      else if (canStore) void store(draft)
    }
    body = (
      <div className="storage-cfg-secret-edit">
        {mode === 'json' ? (
          <textarea
            id={id}
            data-field={path}
            aria-label={label}
            className="storage-cfg-textarea"
            value={draft}
            rows={5}
            spellCheck={false}
            autoComplete="off"
            placeholder={placeholder ?? '{ "type": "service_account", … }'}
            disabled={locked}
            onChange={(event) => setDraft(event.target.value)}
          />
        ) : (
          <Input
            id={id}
            data-field={path}
            aria-label={label}
            className="storage-cfg-mono"
            type={mode === 'secret' ? 'password' : 'text'}
            value={draft}
            placeholder={placeholder ?? (kind === 'empty' ? 'Not set' : 'New value')}
            autoComplete={mode === 'secret' ? 'new-password' : 'off'}
            spellCheck={false}
            disabled={locked}
            onChange={setDraft}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                submit()
              }
            }}
          />
        )}
        {json && !json.ok ? <span className="storage-cfg-warn">{json.reason}</span> : null}
        {json?.ok ? (
          <span className="storage-cfg-secret-status">
            Service account {json.email}
            {json.project ? ` · project ${json.project}` : ''}
          </span>
        ) : null}
        {legacyWorker ? (
          <div className="storage-cfg-notice" role="note">
            This storage worker reads credentials only as plain values or <code>{TEMPLATE_SYNTAX}</code> templates. Update it
            (<code>iii worker update storage</code>) to keep them in the secrets store.
          </div>
        ) : secretsUp === false ? (
          <div className="storage-cfg-notice" role="alert">
            The secrets worker is not running, so this value cannot be kept out of configuration. Start it, or keep the
            value as plain text in the configuration.
          </div>
        ) : null}
        <div className="storage-cfg-secret-actions">
          <span className="storage-cfg-secret-dest">
            {inlineOnly
              ? 'Plain text in configuration'
              : textFirst
                ? 'Kept in configuration'
                : `Stored as secret://${target}`}
          </span>
          {mode === 'json' ? (
            <>
              <input
                ref={fileRef}
                type="file"
                hidden
                accept=".json,application/json"
                onChange={(event) => {
                  void pickFile(event.target.files?.[0])
                  event.target.value = ''
                }}
              />
              <Button type="button" variant="ghost" size="sm" disabled={locked} onClick={() => fileRef.current?.click()}>
                <Upload size={16} aria-hidden />
                Upload file
              </Button>
            </>
          ) : null}
          {editing ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={locked}
              onClick={() => {
                setEditing(false)
                setDraft('')
                setError(null)
              }}
            >
              Cancel
            </Button>
          ) : null}
          {inlineOnly ? (
            <Button type="button" variant="ghost" size="sm" disabled={!draftReady} onClick={() => keepInline(draft)}>
              Keep as plain text
            </Button>
          ) : (
            <>
              {mode === 'identifier' ? (
                <Button
                  type="button"
                  variant={textFirst ? undefined : 'ghost'}
                  size="sm"
                  disabled={locked || !draftReady}
                  onClick={() => keepInline(draft)}
                >
                  Keep in configuration
                </Button>
              ) : null}
              <Button
                type="button"
                variant={textFirst ? 'ghost' : undefined}
                size="sm"
                disabled={locked || !draftReady || !canStore}
                onClick={() => void store(draft)}
              >
                {busy === 'store' ? 'Saving…' : 'Save to secrets'}
              </Button>
            </>
          )}
        </div>
      </div>
    )
  } else if (kind === 'secret') {
    body = row(
      <>
        <span className="storage-cfg-secret-ref">{text}</span>
        <span className={meta === null && secretsUp !== false ? 'storage-cfg-warn' : 'storage-cfg-secret-status'}>
          {legacyWorker ? 'This storage worker cannot resolve references — update it' : secretStatusText(meta, secretsUp)}
        </span>
      </>,
      <>
        {replace}
        {remove}
      </>,
    )
  } else if (kind === 'env') {
    body = row(
      <>
        <span className="storage-cfg-secret-ref">{text}</span>
        <span className="storage-cfg-secret-status">
          {legacyWorker ? 'This storage worker cannot resolve references — update it' : envStatusText(meta, secretsUp)}
        </span>
      </>,
      <>
        {replace}
        {remove}
      </>,
    )
  } else if (kind === 'redacted') {
    body = row(<span className="storage-cfg-secret-status">A stored value the worker keeps hidden</span>, replace)
  } else if (kind === 'template') {
    body = row(
      <>
        <span className="storage-cfg-secret-ref">{text}</span>
        <span className="storage-cfg-secret-status">Expanded from the engine’s environment</span>
      </>,
      <>
        {replace}
        {remove}
      </>,
    )
  } else if (kind === 'literal') {
    const moveButton = canStore ? (
      <Button
        type="button"
        size="sm"
        variant={mode === 'identifier' ? 'ghost' : undefined}
        disabled={locked}
        onClick={() => void store(String(value))}
      >
        {busy === 'store' ? 'Moving…' : 'Move to secrets'}
      </Button>
    ) : null
    if (mode === 'identifier') {
      body = row(
        <>
          <span className="storage-cfg-secret-ref">{text}</span>
          <span className="storage-cfg-secret-status">Kept in configuration</span>
        </>,
        <>
          {moveButton}
          <Button type="button" variant="ghost" size="sm" disabled={locked} onClick={() => startEdit(text)}>
            Edit
          </Button>
          {remove}
        </>,
      )
    } else {
      const account = mode === 'json' ? checkServiceAccountJson(text) : null
      body = row(
        <>
          <Chip tone="warning">Plain text</Chip>
          <span className="storage-cfg-secret-status">
            {account?.ok ? `Service account ${account.email} · ` : ''}
            Saved in configuration, which is meant to be shared.
          </span>
        </>,
        <>
          {moveButton}
          {replace}
          {remove}
        </>,
      )
    }
  } else {
    body = row(
      <Chip tone="warning">Custom value preserved</Chip>,
      <>
        {replace}
        {remove}
      </>,
    )
  }

  return (
    <div className="storage-cfg-secret" data-field={editing || kind === 'empty' ? undefined : path} tabIndex={-1}>
      {body}
      {error ? (
        <span className="storage-cfg-warn" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  )
}
