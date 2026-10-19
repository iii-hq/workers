/**
 * `display.icon` and `display.color`: how apps draw a connection — the same
 * glyphs and tones the iii app offers for an environment. UI only; the
 * worker stores the block and never reads it.
 */

import { Button } from '@iii-dev/console-ui'
import { CONNECTION_ICONS, ConnectionIcon, DEFAULT_CONNECTION_ICON } from '../lib/connection-icons'
import { CONNECTION_COLORS, isHexColor } from './connection'

export function IconPicker({
  value,
  color,
  label,
  onChange,
}: {
  value: string | undefined
  color: string | undefined
  label: string
  onChange: (icon: string | undefined) => void
}) {
  const current = value ?? DEFAULT_CONNECTION_ICON
  const custom = value !== undefined && !CONNECTION_ICONS.includes(value)
  return (
    <div className="db-cfg-icons" role="radiogroup" aria-label={label}>
      {custom ? (
        // Written by another client: kept as stored, shown as the fallback glyph.
        <button type="button" role="radio" aria-checked className="db-cfg-icon" title={`${value} (kept as stored)`}>
          <ConnectionIcon name={value} color={color} />
        </button>
      ) : null}
      {CONNECTION_ICONS.map((icon) => {
        const selected = !custom && icon === current
        return (
          <button
            key={icon}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={icon}
            title={icon}
            className="db-cfg-icon"
            onClick={() => onChange(icon === DEFAULT_CONNECTION_ICON ? undefined : icon)}
          >
            <ConnectionIcon name={icon} color={selected ? color : undefined} />
          </button>
        )
      })}
    </div>
  )
}

export function ColorPicker({
  value,
  label,
  onChange,
}: {
  value: string | undefined
  label: string
  onChange: (color: string | undefined) => void
}) {
  const normalized = value?.toUpperCase()
  const custom = normalized !== undefined && !CONNECTION_COLORS.includes(normalized)
  return (
    <div className="db-cfg-swatches" role="radiogroup" aria-label={label}>
      <Button type="button" variant="ghost" size="sm" aria-pressed={value === undefined} onClick={() => onChange(undefined)}>
        None
      </Button>
      {CONNECTION_COLORS.map((color) => (
        <button
          key={color}
          type="button"
          role="radio"
          aria-checked={normalized === color}
          aria-label={color}
          title={color}
          className="db-cfg-swatch"
          style={{ background: color }}
          onClick={() => onChange(color)}
        />
      ))}
      <label className="db-cfg-swatch-custom" title="Custom colour">
        <input
          type="color"
          aria-label="Custom colour"
          value={isHexColor(value) ? value : '#22C55E'}
          onChange={(event) => onChange(event.target.value.toUpperCase())}
        />
        {custom ? <span className="db-cfg-technical">{value}</span> : <span>Custom</span>}
      </label>
    </div>
  )
}
