/**
 * Small shared widgets for the fleet page: click-to-copy, the env KV row
 * editor (composer + create dialog share it), and the S-code error card
 * (create dialog + file ops share it).
 */

import { Badge, Input } from '@iii-dev/console-ui'
import { useCopyFlash } from '../lib/clipboard'
import type { SandboxError } from './errors'
import { CopyIcon } from './icons'

export function CopyButton({
  text,
  label,
  title,
}: {
  text: string
  /** Optional visible label next to the glyph. */
  label?: string
  title?: string
}) {
  const { state, copy } = useCopyFlash(text)
  return (
    <button
      type="button"
      className={`cr-page-copy${state === 'idle' ? '' : ` ${state}`}`}
      onClick={copy}
      title={title ?? 'copy'}
    >
      <CopyIcon aria-hidden />
      {label ? <span>{label}</span> : null}
      {state === 'copied' ? <span className="flash">copied</span> : null}
      {state === 'failed' ? <span className="flash">copy failed</span> : null}
    </button>
  )
}

export interface EnvRow {
  key: string
  value: string
}

export function EnvRowsEditor({
  rows,
  onChange,
  disabled,
}: {
  rows: EnvRow[]
  onChange(next: EnvRow[]): void
  disabled?: boolean
}) {
  const update = (index: number, patch: Partial<EnvRow>) => {
    const next = rows.slice()
    next[index] = { ...next[index], ...patch }
    onChange(next)
  }
  return (
    <div className="cr-page-envrows">
      {rows.map((row, index) => (
        // Positional rows with no natural id — index keys are honest here.
        // biome-ignore lint/suspicious/noArrayIndexKey: positional rows
        <div className="cr-page-envrow" key={index}>
          <Input
            value={row.key}
            onChange={(key) => update(index, { key })}
            placeholder="KEY"
            preserveCase
            aria-label={`env key ${index + 1}`}
            disabled={disabled}
          />
          <span className="cr-page-envrow-eq" aria-hidden>
            =
          </span>
          <Input
            value={row.value}
            onChange={(value) => update(index, { value })}
            placeholder="value"
            preserveCase
            aria-label={`env value ${index + 1}`}
            disabled={disabled}
          />
          <button
            type="button"
            className="cr-page-linkish"
            aria-label={`remove env row ${index + 1}`}
            onClick={() => onChange(rows.filter((_, i) => i !== index))}
            disabled={disabled}
          >
            ×
          </button>
        </div>
      ))}
      <button
        type="button"
        className="cr-page-linkish"
        onClick={() => onChange([...rows, { key: '', value: '' }])}
        disabled={disabled}
      >
        + env var
      </button>
    </div>
  )
}

/** The daemon's flat S-code error, rendered whole: code, message, fix
 *  note, and whether retrying can help at all. */
export function SandboxErrorCard({ error }: { error: SandboxError }) {
  return (
    <div className="cr-page-errcard" role="alert">
      <div className="cr-page-errcard-head">
        {error.code ? <code className="cr-page-errcode">{error.code}</code> : null}
        {error.retryable !== null ? (
          <Badge variant={error.retryable ? 'warn' : 'alert'}>
            {error.retryable ? 'retryable' : 'not retryable'}
          </Badge>
        ) : null}
      </div>
      <div className="cr-page-errcard-msg">{error.message}</div>
      {error.fix_note ? (
        <div className="cr-page-errcard-fix">{error.fix_note}</div>
      ) : null}
    </div>
  )
}
