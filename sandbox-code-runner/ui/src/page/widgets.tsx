/**
 * Small shared widgets for the fleet page: click-to-copy, the env KV row
 * editor (composer + create dialog share it), and the S-code error card
 * (create dialog + file ops share it).
 */

import { Badge, Button, IconButton, Input, StatusPanel } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { uiClasses } from '@iii-dev/console-ui/ui-classes'
import { Copy, Plus, X } from 'lucide-react'
import type { SandboxError } from './errors'

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
  // The accessible name stays stable across the flash — the outcome is
  // announced through the persistent live region instead.
  return (
    <button
      type="button"
      className={`cr-page-copy${state === 'idle' ? '' : ` ${state}`}`}
      onClick={copy}
      title={title ?? 'copy'}
      aria-label={label ?? title ?? 'copy'}
    >
      <Copy size={16} aria-hidden />
      {label ? <span>{label}</span> : null}
      {state === 'copied' ? (
        <span className={uiClasses.eyebrow} aria-hidden>
          copied
        </span>
      ) : null}
      {state === 'failed' ? (
        <span className={uiClasses.eyebrow} aria-hidden>
          copy failed
        </span>
      ) : null}
      <span className="cr-page-visually-hidden" role="status" aria-live="polite">
        {state === 'copied' ? 'copied' : state === 'failed' ? 'copy failed' : ''}
      </span>
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
          <IconButton
            variant="ghost"
            label={`remove env row ${index + 1}`}
            onClick={() => onChange(rows.filter((_, i) => i !== index))}
            disabled={disabled}
          >
            <X size={16} aria-hidden />
          </IconButton>
        </div>
      ))}
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onChange([...rows, { key: '', value: '' }])}
        disabled={disabled}
      >
        <Plus size={16} aria-hidden /> env var
      </Button>
    </div>
  )
}

/** The daemon's flat S-code error, rendered whole: code, message, fix
 *  note, and whether retrying can help at all. */
export function SandboxErrorCard({ error }: { error: SandboxError }) {
  return (
    <StatusPanel
      variant="alert"
      role="alert"
      headline={
        <>
          {error.code ? <code className="cr-page-errcode">{error.code}</code> : null}
          {error.retryable !== null ? (
            <Badge variant={error.retryable ? 'warn' : 'alert'}>
              {error.retryable ? 'retryable' : 'not retryable'}
            </Badge>
          ) : null}{' '}
          {error.message}
        </>
      }
      detail={error.fix_note}
    />
  )
}
