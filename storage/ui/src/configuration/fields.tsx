/** The configuration form's field rows: a label, the control, an error and a hint. */

import { Checkbox, IconButton, Input, Select } from '@iii-dev/console-ui'
import { Eye, EyeOff } from 'lucide-react'
import { type ReactNode, useState } from 'react'

export function Field({
  label,
  hint,
  error,
  className = '',
  children,
}: {
  label: ReactNode
  hint?: ReactNode
  error?: string
  className?: string
  children: ReactNode
}) {
  return (
    <div className={`storage-cfg-field${className ? ` ${className}` : ''}`}>
      <div className="storage-cfg-field-label">
        <span>{label}</span>
      </div>
      {children}
      {error ? (
        <p className="storage-cfg-error" role="alert">
          {error}
        </p>
      ) : null}
      {hint ? <p className="storage-cfg-hint">{hint}</p> : null}
    </div>
  )
}

export function TextField({
  id,
  path,
  label,
  value,
  placeholder,
  hint,
  error,
  type = 'text',
  onChange,
}: {
  id: string
  path: string
  label: string
  value: string
  placeholder?: string
  hint?: ReactNode
  error?: string
  type?: 'text' | 'password'
  onChange: (value: string) => void
}) {
  const [revealed, setRevealed] = useState(false)
  const secret = type === 'password'
  return (
    <Field label={<label htmlFor={id}>{label}</label>} hint={hint} error={error}>
      <div className="storage-cfg-input-row">
        <Input
          id={id}
          data-field={path}
          name={id}
          type={secret && !revealed ? 'password' : 'text'}
          value={value}
          spellCheck={false}
          autoComplete="off"
          placeholder={placeholder}
          onChange={onChange}
        />
        {secret ? (
          <IconButton
            label={`${revealed ? 'Hide' : 'Show'} ${label}`}
            onClick={() => setRevealed((current) => !current)}
          >
            {revealed ? <EyeOff /> : <Eye />}
          </IconButton>
        ) : null}
      </div>
    </Field>
  )
}

export function SelectField<T extends string>({
  id,
  path,
  label,
  value,
  options,
  hint,
  error,
  onChange,
}: {
  id: string
  path: string
  label: string
  value: T
  options: Array<{ value: T; label: string; description?: string }>
  hint?: ReactNode
  error?: string
  onChange: (value: T) => void
}) {
  return (
    <Field label={<label htmlFor={id}>{label}</label>} hint={hint} error={error}>
      <Select<T> id={id} data-field={path} name={id} value={value} options={options} onChange={onChange} />
    </Field>
  )
}

export function CheckField({
  id,
  path,
  label,
  hint,
  checked,
  onChange,
}: {
  id: string
  path: string
  label: string
  hint?: ReactNode
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <div className="storage-cfg-check-field">
      <Checkbox
        id={id}
        data-field={path}
        name={id}
        label={label}
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      {hint ? <p className="storage-cfg-hint">{hint}</p> : null}
    </div>
  )
}
