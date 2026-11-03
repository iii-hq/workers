/**
 * A Cloud API credential: access token, app secret or webhook verify token.
 *
 * A new value is not written into configuration: "Save to secrets" stores it
 * in the secrets worker under `target` and the field becomes `secret://NAME`.
 * Afterwards the field shows the reference and whether the store has it and
 * shares it with WhatsApp, never the value. A plain-text value (from before
 * references, or typed into YAML) is flagged, with a one-click move into the
 * store; `env://NAME` is shown as such. When the secrets worker is not running
 * the field says so and keeps the value inline only on an explicit choice.
 */

import { Badge, Button, Input, type JsonValue, type SettingsFieldControlProps, type Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useState, type ReactNode } from 'react'
import { envRefName, getSecret, referenceStatus, secretRefName, storeSecret, type SecretMeta } from './secrets'

export function CredentialControl({ host, control, label, value, target, onChange }: {
  host: Host
  control: SettingsFieldControlProps
  label: string
  value: JsonValue | undefined
  /** The secret a new value is stored as. */
  target: string
  onChange(next: string | undefined): void
}) {
  const text = typeof value === 'string' ? value.trim() : ''
  const secretName = secretRefName(value)
  const envName = envRefName(value)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // undefined: still reading · null: nothing by that name
  const [meta, setMeta] = useState<SecretMeta | null | undefined>(undefined)
  // undefined: unknown yet · false: the secrets worker is not running
  const [secretsUp, setSecretsUp] = useState<boolean | undefined>(undefined)
  const [reload, setReload] = useState(0)

  // The referenced secret or variable, else the name a new value would take
  // (which also tells whether the secrets worker is running).
  const probeName = secretName ?? envName ?? target
  const probeStore = envName && !secretName ? 'env' : undefined
  useEffect(() => {
    let alive = true
    setMeta(undefined)
    getSecret(host, probeName, probeStore)
      .then((found) => { if (alive) { setSecretsUp(found !== undefined); setMeta(found ?? null) } })
      .catch(() => { if (alive) { setSecretsUp(true); setMeta(null) } })
    return () => { alive = false }
  }, [host, probeName, probeStore, reload])

  const store = async (secret: string) => {
    setBusy(true)
    setError(null)
    try {
      onChange(await storeSecret(host, { name: target, value: secret, description: `WhatsApp Cloud API ${label.toLowerCase()}, stored from the console` }))
      setEditing(false)
      setDraft('')
      setReload((n) => n + 1)
    } catch (cause) {
      setError(errorMessage(cause))
    } finally {
      setBusy(false)
    }
  }
  const keepInline = () => {
    onChange(draft.trim() || undefined)
    setEditing(false)
    setDraft('')
    setError(null)
  }
  const startEdit = () => { setEditing(true); setDraft(''); setError(null) }
  const replace = <Button type="button" variant="ghost" disabled={busy} onClick={startEdit}>Replace</Button>
  const remove = <Button type="button" variant="ghost" disabled={busy} onClick={() => onChange(undefined)}>Remove</Button>
  const row = (copy: ReactNode, buttons: ReactNode) => <div className="wa-ui-credential-row" data-field={control['data-field']} tabIndex={-1}>
    <span className="wa-ui-credential-copy">{copy}</span>
    <span className="wa-ui-credential-buttons">{buttons}</span>
  </div>

  let body: ReactNode
  if (editing || text === '') {
    const ready = draft.trim() !== ''
    const inlineOnly = secretsUp === false
    // Replacing the referenced secret rotates it in place, for the running worker too.
    const rotating = editing && secretName === target
    body = <div className="wa-ui-credential-edit">
      <Input {...control} aria-label={label} className="wa-ui-mono" type="password" value={draft} placeholder={text === '' ? 'Not set' : 'New value'}
        autoComplete="new-password" spellCheck={false} disabled={busy} onChange={setDraft}
        onKeyDown={(event) => {
          if (event.key !== 'Enter') return
          event.preventDefault()
          if (!ready || busy) return
          if (inlineOnly) keepInline()
          else if (secretsUp) void store(draft)
        }} />
      {inlineOnly && <p className="wa-ui-note wa-ui-field-error" role="alert">The secrets worker is not running, so this value cannot be kept out of configuration. Start it, or keep the value as plain text in the configuration.</p>}
      <div className="wa-ui-credential-actions">
        <span className="wa-ui-note">{inlineOnly ? 'Plain text in configuration' : <>Stored as <span className="wa-ui-mono">secret://{target}</span>{rotating ? '. WhatsApp uses the new value once it is saved.' : ''}</>}</span>
        {editing && <Button type="button" variant="ghost" disabled={busy} onClick={() => { setEditing(false); setDraft(''); setError(null) }}>Cancel</Button>}
        {inlineOnly
          ? <Button type="button" variant="ghost" disabled={!ready} onClick={keepInline}>Keep as plain text</Button>
          : <Button type="button" disabled={busy || !ready || secretsUp !== true} onClick={() => void store(draft)}>{busy ? 'Saving…' : 'Save to secrets'}</Button>}
      </div>
    </div>
  } else if (secretName || envName) {
    const status = referenceStatus(secretName ? 'secret' : 'env', meta, secretsUp)
    body = row(<>
      <span className="wa-ui-mono">{text}</span>
      <span className={status.warn ? 'wa-ui-note wa-ui-field-error' : 'wa-ui-note'}>{status.text}</span>
    </>, <>{replace}{remove}</>)
  } else {
    body = row(<>
      <Badge>Plain text</Badge>
      <span className="wa-ui-note">Saved in configuration, which is meant to be shared.</span>
    </>, <>
      {secretsUp && <Button type="button" disabled={busy} onClick={() => void store(text)}>{busy ? 'Moving…' : 'Move to secrets'}</Button>}
      {replace}{remove}
    </>)
  }
  return <div className="wa-ui-credential">
    {body}
    {error && <p className="wa-ui-note wa-ui-field-error" role="alert">{error}</p>}
  </div>
}
