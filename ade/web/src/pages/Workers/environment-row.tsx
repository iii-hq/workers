import { Trash2, Undo2 } from 'lucide-react'
import { useId } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { IconButton } from '@/components/ui/IconButton'
import { Input } from '@/components/ui/Input'
import { SettingsRow } from '@/components/ui/Settings'
import { type EnvDraft, isSecretValue, MASK } from './model'

export function EnvironmentRow({
  row,
  error,
  onChange,
  onRemove,
}: {
  row: EnvDraft
  error: string | undefined
  onChange: (next: Partial<EnvDraft>) => void
  onRemove: () => void
}) {
  const errorId = useId()
  const describedBy = error ? errorId : undefined
  const secret = row.secret || isSecretValue(row.key, row.value)
  return (
    <SettingsRow
      className="wk-env"
      data-removed={row.removed || undefined}
      label={
        row.isNew ? (
          <Input
            aria-label="Variable name"
            aria-invalid={!!error}
            aria-describedby={describedBy}
            className="wk-mono"
            placeholder="NAME"
            value={row.key}
            onChange={(key) => onChange({ key })}
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
          />
        ) : (
          <span className="wk-mono wk-env-key">{row.key}</span>
        )
      }
      meta={
        error ? (
          <span id={errorId} className="wk-field-error" role="alert">
            {error}
          </span>
        ) : undefined
      }
      control={
        row.secret && !row.replacing ? (
          <span className="wk-secret">
            <span className="wk-mono wk-faint">{MASK}</span>
            <Badge>secret</Badge>
            <Button
              variant="ghost"
              size="sm"
              disabled={row.removed}
              onClick={() => onChange({ replacing: true, value: '' })}
            >
              Replace
            </Button>
          </span>
        ) : (
          <span className="wk-env-value">
            <Input
              aria-label={`${row.key || 'New variable'} value`}
              aria-invalid={!!error}
              aria-describedby={describedBy}
              className="wk-mono wk-value"
              type={secret ? 'password' : 'text'}
              placeholder={secret ? 'New secret value' : 'value'}
              value={row.value}
              disabled={row.removed}
              onChange={(value) => onChange({ value })}
              autoComplete="off"
              spellCheck={false}
            />
            {row.secret && row.replacing ? (
              <IconButton
                label={`Cancel replacing ${row.key}`}
                variant="ghost"
                onClick={() => onChange({ replacing: false, value: '' })}
              >
                <Undo2 />
              </IconButton>
            ) : null}
          </span>
        )
      }
      action={
        row.removed ? (
          <IconButton
            label={`Keep ${row.key}`}
            variant="ghost"
            onClick={() => onChange({ removed: false })}
          >
            <Undo2 />
          </IconButton>
        ) : (
          <IconButton
            label={`Remove ${row.key || 'variable'}`}
            variant="ghost"
            className="wk-danger"
            onClick={onRemove}
          >
            <Trash2 />
          </IconButton>
        )
      }
    />
  )
}
