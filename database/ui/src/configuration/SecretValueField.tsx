/**
 * A credential field — password, client key, SSH key or passphrase.
 *
 * The value is never written into configuration: "Save" stores it in the
 * secrets worker under the connection's derived name and the field becomes
 * `secret://NAME`. What the field shows afterwards is the reference and the
 * store's masked hint, never the value. A plain-text value that predates
 * this (or was typed into YAML by hand) is flagged, with a one-click move
 * into the store. When the secrets worker is not running the field says so
 * and, only on an explicit choice, keeps the value inline.
 */

import {
  Button,
  Chip,
  type Host,
  Input,
  type JsonValue,
  SettingsField,
  type SettingsFieldControlProps,
} from '@iii-dev/console-ui'
import { Upload } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { errText } from '../lib/errors'
import { getSecret, type SecretMeta, secretRefName, storeSecret } from '../lib/secrets'
import { isPem, looksLikeKeyPath, valueKind } from './connection'

export interface SecretValueFieldProps {
  host: Host
  id: string
  /** Configuration path, for deep links (`databases.shop.password`). */
  field: string
  label: string
  description?: ReactNode
  error?: ReactNode
  value: JsonValue | undefined
  /** Where a new value is stored — derived from the handle, already unique. */
  secretName: string
  /** The connection handle, for the secret's description. */
  connection: string
  /** PEM / OpenSSH material: a textarea with a file upload instead of one line. */
  multiline?: boolean
  placeholder?: string
  onChange: (next: string | undefined) => void
}

export function SecretValueField(props: SecretValueFieldProps) {
  return (
    <SettingsField
      id={props.id}
      field={props.field}
      label={props.label}
      description={props.description}
      error={props.error}
      layout="stacked"
      controlSize="full"
      renderControl={(controlProps) => <SecretValueControl {...controlProps} {...props} />}
    />
  )
}

type Busy = 'store' | null

function SecretValueControl({
  id,
  'data-field': dataField,
  'aria-invalid': ariaInvalid,
  'aria-describedby': ariaDescribedBy,
  host,
  label,
  value,
  secretName,
  connection,
  multiline,
  placeholder,
  onChange,
}: SettingsFieldControlProps & SecretValueFieldProps) {
  const kind = valueKind(value)
  const text = typeof value === 'string' ? value.trim() : ''
  const refName = secretRefName(value)
  // Replacing a referenced value rotates the same secret, so the reference
  // in configuration (and anything else reading it) stays valid.
  const target = refName ?? secretName
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

  // biome-ignore lint/correctness/useExhaustiveDependencies: `reload` re-reads after this field stored a value
  useEffect(() => {
    let alive = true
    setMeta(undefined)
    getSecret(host, target)
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
  }, [host, target, reload])

  const store = async (secretValue: string) => {
    setBusy('store')
    setError(null)
    try {
      const ref = await storeSecret(host, {
        name: target,
        value: secretValue,
        description: `${label} for database connection “${connection}”, stored from the console`,
      })
      onChange(ref)
      setEditing(false)
      setDraft('')
      setReload((n) => n + 1)
    } catch (cause) {
      setError(errText(cause))
    } finally {
      setBusy(null)
    }
  }

  const pickFile = async (file: File | undefined) => {
    if (!file) return
    try {
      setDraft(await file.text())
      setError(null)
    } catch (cause) {
      setError(errText(cause))
    }
  }

  const locked = busy !== null
  const replace = (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      disabled={locked}
      onClick={() => {
        setEditing(true)
        setDraft('')
        setError(null)
      }}
    >
      Replace
    </Button>
  )
  const remove = (
    <Button type="button" variant="ghost" size="sm" disabled={locked} onClick={() => onChange(undefined)}>
      Remove
    </Button>
  )

  let body: ReactNode
  if (editing || kind === 'empty') {
    const draftReady = draft.trim() !== ''
    // A key file on the engine host is named, not pasted: a path is not a
    // secret, so it stays in configuration as typed.
    const draftIsPath = multiline === true && looksLikeKeyPath(draft)
    const notPem = multiline && draftReady && !draftIsPath && !isPem(draft)
    body = (
      <div className="db-cfg-secret-edit">
        {multiline ? (
          <textarea
            id={id}
            data-field={dataField}
            aria-invalid={ariaInvalid}
            aria-describedby={ariaDescribedBy}
            aria-label={label}
            className="db-cfg-pem"
            value={draft}
            rows={5}
            spellCheck={false}
            autoComplete="off"
            placeholder={placeholder ?? '-----BEGIN PRIVATE KEY----- … or a path on the engine host'}
            disabled={locked}
            onChange={(event) => setDraft(event.target.value)}
          />
        ) : (
          <Input
            id={id}
            data-field={dataField}
            aria-invalid={ariaInvalid}
            aria-describedby={ariaDescribedBy}
            aria-label={label}
            className="db-cfg-technical db-cfg-grow"
            type="password"
            value={draft}
            placeholder={placeholder ?? (kind === 'empty' ? 'Not set' : 'New value')}
            autoComplete="new-password"
            spellCheck={false}
            disabled={locked}
            onChange={setDraft}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && draftReady && secretsUp !== false) {
                event.preventDefault()
                void store(draft)
              }
            }}
          />
        )}
        {notPem ? <span className="db-cfg-warn">This does not look like a PEM or OpenSSH key.</span> : null}
        {secretsUp === false && !draftIsPath ? (
          <div className="db-cfg-notice" role="alert">
            The secrets worker is not running, so this value cannot be kept out of configuration. Start it, or keep the
            value as plain text in the configuration file.
          </div>
        ) : null}
        <div className="db-cfg-secret-actions">
          <span className="db-cfg-secret-dest">
            {draftIsPath
              ? 'A path on the engine host, kept in configuration'
              : secretsUp === false
                ? 'Plain text in configuration'
                : `Stored as secret://${target}`}
          </span>
          {multiline ? (
            <>
              <input
                ref={fileRef}
                type="file"
                hidden
                accept=".pem,.key,.crt,.cer,.ppk,text/plain"
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
          {draftIsPath ? (
            <Button
              type="button"
              size="sm"
              disabled={locked}
              onClick={() => {
                onChange(draft.trim())
                setEditing(false)
                setDraft('')
              }}
            >
              Use path
            </Button>
          ) : secretsUp === false ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={!draftReady}
              onClick={() => {
                onChange(draft)
                setEditing(false)
                setDraft('')
              }}
            >
              Keep as plain text
            </Button>
          ) : (
            <Button
              type="button"
              size="sm"
              disabled={locked || !draftReady || secretsUp === undefined}
              onClick={() => void store(draft)}
            >
              {busy === 'store' ? 'Saving…' : 'Save to secrets'}
            </Button>
          )}
        </div>
      </div>
    )
  } else if (kind === 'secret') {
    body = (
      <div className="db-cfg-secret-row">
        <span className="db-cfg-secret-copy">
          <span className="db-cfg-secret-ref">{text}</span>
          <span className={meta === null ? 'db-cfg-warn' : 'db-cfg-secret-status'}>
            {meta === undefined
              ? secretsUp === false
                ? 'The secrets worker is not running'
                : 'Reading the secrets store…'
              : meta
                ? ['Stored in secrets', meta.hint].filter(Boolean).join(' · ')
                : 'Not in the secrets store — the connection cannot resolve it'}
          </span>
        </span>
        <span className="db-cfg-secret-buttons">
          {replace}
          {remove}
        </span>
      </div>
    )
  } else if (kind === 'env') {
    body = (
      <div className="db-cfg-secret-row">
        <span className="db-cfg-secret-copy">
          <span className="db-cfg-secret-ref">{text}</span>
          <span className="db-cfg-secret-status">An environment variable the secrets worker reads</span>
        </span>
        <span className="db-cfg-secret-buttons">
          {replace}
          {remove}
        </span>
      </div>
    )
  } else if (kind === 'redacted') {
    body = (
      <div className="db-cfg-secret-row">
        <span className="db-cfg-secret-copy">
          <span className="db-cfg-secret-status">A stored value the worker keeps hidden</span>
        </span>
        <span className="db-cfg-secret-buttons">{replace}</span>
      </div>
    )
  } else if (kind === 'template') {
    body = (
      <div className="db-cfg-secret-row">
        <span className="db-cfg-secret-copy">
          <span className="db-cfg-secret-ref">{text}</span>
          <span className="db-cfg-secret-status">Expanded from the engine’s environment</span>
        </span>
        <span className="db-cfg-secret-buttons">
          {replace}
          {remove}
        </span>
      </div>
    )
  } else if (kind === 'literal' && multiline && looksLikeKeyPath(text)) {
    body = (
      <div className="db-cfg-secret-row">
        <span className="db-cfg-secret-copy">
          <span className="db-cfg-secret-ref">{text}</span>
          <span className="db-cfg-secret-status">A file on the engine host; the worker reads the key from it</span>
        </span>
        <span className="db-cfg-secret-buttons">
          {replace}
          {remove}
        </span>
      </div>
    )
  } else if (kind === 'literal') {
    body = (
      <div className="db-cfg-secret-row">
        <span className="db-cfg-secret-copy">
          <Chip tone="warning">Plain text</Chip>
          <span className="db-cfg-secret-status">Saved in configuration, which is meant to be shared.</span>
        </span>
        <span className="db-cfg-secret-buttons">
          {secretsUp !== false ? (
            <Button type="button" size="sm" disabled={locked} onClick={() => void store(String(value))}>
              {busy === 'store' ? 'Moving…' : 'Move to secrets'}
            </Button>
          ) : null}
          {replace}
          {remove}
        </span>
      </div>
    )
  } else {
    body = (
      <div className="db-cfg-secret-row">
        <span className="db-cfg-secret-copy">
          <Chip tone="warning">Custom value preserved</Chip>
        </span>
        <span className="db-cfg-secret-buttons">
          {replace}
          {remove}
        </span>
      </div>
    )
  }

  return (
    <div className="db-cfg-secret">
      {body}
      {error ? (
        <span className="db-cfg-test-fail" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  )
}
