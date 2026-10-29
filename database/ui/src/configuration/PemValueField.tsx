/**
 * A certificate field (CA bundle, client certificate): a path on the engine
 * host, an inline PEM pasted or uploaded here, or a `secret://` / `env://`
 * reference. Certificates are public, so inline PEM is kept in configuration
 * as-is; the private half of a client certificate goes through
 * `SecretValueField` instead.
 */

import { Button, Input, type JsonValue, SettingsField, type SettingsFieldControlProps } from '@iii-dev/console-ui'
import { ClipboardPaste, Upload } from 'lucide-react'
import { type ReactNode, useRef, useState } from 'react'
import { errText } from '../lib/errors'
import { isPem, pemSummary } from './connection'

export interface PemValueFieldProps {
  id: string
  field: string
  label: string
  description?: ReactNode
  error?: ReactNode
  value: JsonValue | undefined
  pathPlaceholder: string
  onChange: (next: string | undefined) => void
}

export function PemValueField(props: PemValueFieldProps) {
  return (
    <SettingsField
      id={props.id}
      field={props.field}
      label={props.label}
      description={props.description}
      error={props.error}
      layout="stacked"
      controlSize="full"
      renderControl={(controlProps) => <PemValueControl {...controlProps} {...props} />}
    />
  )
}

function PemValueControl({
  id,
  'data-field': dataField,
  'aria-invalid': ariaInvalid,
  'aria-describedby': ariaDescribedBy,
  label,
  value,
  pathPlaceholder,
  onChange,
}: SettingsFieldControlProps & PemValueFieldProps) {
  const text = typeof value === 'string' ? value : ''
  const inline = isPem(text)
  const [pasting, setPasting] = useState(false)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const pickFile = async (file: File | undefined) => {
    if (!file) return
    try {
      const content = await file.text()
      if (!isPem(content)) {
        setError(`${file.name} is not a PEM file.`)
        return
      }
      setError(null)
      setPasting(false)
      onChange(content)
    } catch (cause) {
      setError(errText(cause))
    }
  }

  const upload = (
    <>
      <input
        ref={fileRef}
        type="file"
        hidden
        accept=".pem,.crt,.cer,text/plain"
        onChange={(event) => {
          void pickFile(event.target.files?.[0])
          event.target.value = ''
        }}
      />
      <Button type="button" variant="ghost" size="sm" onClick={() => fileRef.current?.click()}>
        <Upload size={16} aria-hidden />
        Upload
      </Button>
    </>
  )

  if (pasting) {
    return (
      <div className="db-cfg-secret-edit">
        <textarea
          id={id}
          data-field={dataField}
          aria-invalid={ariaInvalid}
          aria-describedby={ariaDescribedBy}
          aria-label={label}
          className="db-cfg-pem"
          rows={5}
          value={draft}
          spellCheck={false}
          placeholder="-----BEGIN CERTIFICATE-----"
          onChange={(event) => setDraft(event.target.value)}
        />
        {draft.trim() && !isPem(draft) ? <span className="db-cfg-warn">Not a PEM certificate.</span> : null}
        <div className="db-cfg-secret-actions">
          <span className="db-cfg-secret-dest">Kept inline in configuration</span>
          <Button type="button" variant="ghost" size="sm" onClick={() => setPasting(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={!isPem(draft)}
            onClick={() => {
              onChange(draft.trim())
              setPasting(false)
              setDraft('')
            }}
          >
            Use certificate
          </Button>
        </div>
      </div>
    )
  }

  return (
    <div className="db-cfg-secret">
      {inline ? (
        <div className="db-cfg-secret-row">
          <span className="db-cfg-secret-copy">
            <span className="db-cfg-secret-ref" data-field={dataField}>
              {pemSummary(text)}
            </span>
            <span className="db-cfg-secret-status">
              Inline in configuration · {text.trim().split('\n').length} lines
            </span>
          </span>
          <span className="db-cfg-secret-buttons">
            <Button type="button" variant="ghost" size="sm" onClick={() => setPasting(true)}>
              Replace
            </Button>
            {upload}
            <Button type="button" variant="ghost" size="sm" onClick={() => onChange(undefined)}>
              Remove
            </Button>
          </span>
        </div>
      ) : (
        <div className="db-cfg-url-row">
          <Input
            id={id}
            data-field={dataField}
            aria-invalid={ariaInvalid}
            aria-describedby={ariaDescribedBy}
            aria-label={label}
            className="db-cfg-technical db-cfg-grow"
            type="text"
            value={text}
            placeholder={pathPlaceholder}
            spellCheck={false}
            autoComplete="off"
            onChange={(next) => onChange(next === '' ? undefined : next)}
          />
          <Button type="button" variant="ghost" size="sm" onClick={() => setPasting(true)}>
            <ClipboardPaste size={16} aria-hidden />
            Paste PEM
          </Button>
          {upload}
        </div>
      )}
      {error ? (
        <span className="db-cfg-test-fail" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  )
}
