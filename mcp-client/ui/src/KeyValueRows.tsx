import { Button, IconButton, Input, type InputProps, type SecretKeyFieldProps } from '@iii-dev/console-ui'
import { Plus, X } from 'lucide-react'
import { type ComponentType, useId, useState } from 'react'
import { type KV, REFERENCE, secretVaultName } from './config-model'
import { looksSecretName } from './secret-names'

const CONSUMERS = ['mcp-client']
const REFERENCE_ONLY = 'Use secret://NAME or env://NAME. Store the value in the secrets worker; never paste it here.'
const WORDS = {
  header: { group: 'Headers', name: 'Header name', value: 'Header value', remove: 'Remove header', add: 'Add header' },
  env: {
    group: 'Environment',
    name: 'Variable name',
    value: 'Variable value',
    remove: 'Remove variable',
    add: 'Add variable',
  },
}

/** The input of a secret position on a console without `SecretKeyField`: it takes only `''` or a whole
 * reference. Any other text stays here, so a pasted value never reaches the draft. */
function ReferenceInput({ value, onChange, ...input }: InputProps) {
  const id = useId()
  const [stray, setStray] = useState<string | null>(null)
  return (
    <>
      <Input
        {...input}
        value={stray ?? value}
        aria-invalid={stray !== null || input['aria-invalid'] || undefined}
        aria-describedby={
          [input['aria-describedby'], stray === null ? null : id].filter(Boolean).join(' ') || undefined
        }
        onChange={(next) => {
          const ok = next === '' || REFERENCE.test(next)
          setStray(ok ? null : next)
          if (ok) onChange(next)
        }}
      />
      {stray === null ? null : (
        <p id={id} className="mcp-ui-input-error">
          {REFERENCE_ONLY}
        </p>
      )}
    </>
  )
}

/** A secret position: the console's `SecretKeyField` under the vault name `vault`, else the reference-only
 * input. `environment`: the worker resolves `env://` through the secrets worker too. */
export function SecretValue({
  SecretKeyField,
  vault,
  label,
  value,
  onChange,
  disabled,
  ...input
}: Omit<InputProps, 'disabled'> & {
  SecretKeyField?: ComponentType<SecretKeyFieldProps>
  vault: string
  label: string
  disabled: boolean
}) {
  if (!SecretKeyField) return <ReferenceInput {...input} value={value} onChange={onChange} />
  return (
    <SecretKeyField
      name={vault}
      label={label}
      value={value || undefined}
      consumers={CONSUMERS}
      environment
      disabled={disabled}
      onChange={(next) => onChange(next ?? '')}
    />
  )
}

/** Name and value rows of `headers` or `env`. A name that looks like a credential makes its value a secret
 * position; any other value is a plain input. */
export function KeyValueRows({
  kind,
  rows,
  onChange,
  server,
  SecretKeyField,
  errors,
}: {
  kind: 'header' | 'env'
  rows: KV[]
  onChange: (rows: KV[]) => void
  server: string
  SecretKeyField?: ComponentType<SecretKeyFieldProps>
  errors: Record<number, string>
}) {
  const id = useId()
  const words = WORDS[kind]
  const set = (i: number, change: Partial<KV>) =>
    onChange(rows.map((row, j) => (j === i ? { ...row, ...change } : row)))
  return (
    <fieldset className="mcp-ui-kv" aria-label={words.group}>
      {rows.map((row, i) => {
        const secret = looksSecretName(row.name)
        const error = errors[i]
        const value = {
          'aria-label': words.value,
          'aria-invalid': error ? (true as const) : undefined,
          'aria-describedby': error ? `${id}-${i}` : undefined,
          value: row.value,
          onChange: (next: string) => set(i, { value: next }),
        }
        return (
          <div key={i} className="mcp-ui-kv-row" data-card={(secret && SecretKeyField !== undefined) || undefined}>
            <Input
              className="mcp-ui-kv-name"
              aria-label={words.name}
              value={row.name}
              onChange={(next) => set(i, { name: next })}
            />
            {secret ? (
              <SecretValue
                {...value}
                SecretKeyField={SecretKeyField}
                vault={secretVaultName(server, row.name)}
                label={row.name}
                disabled={server === ''}
              />
            ) : (
              <Input {...value} />
            )}
            <IconButton
              variant="ghost"
              label={`${words.remove} ${row.name}`.trim()}
              onClick={() => onChange(rows.filter((_, j) => j !== i))}
            >
              <X size={16} aria-hidden />
            </IconButton>
            {error ? (
              <p id={`${id}-${i}`} className="mcp-ui-input-error">
                {error}
              </p>
            ) : null}
          </div>
        )
      })}
      <Button variant="ghost" size="sm" onClick={() => onChange([...rows, { name: '', value: '' }])}>
        <Plus size={16} aria-hidden />
        {words.add}
      </Button>
    </fieldset>
  )
}
