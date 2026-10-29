/**
 * `display.icon` and `display.color`: how apps draw a bucket — the same
 * glyphs and tones the iii app offers for an environment. UI only; the
 * worker stores the block and never reads it.
 */

import { Button } from '@iii-dev/console-ui'
import { BUCKET_ICONS, BucketIcon, DEFAULT_BUCKET_ICON } from '../lib/bucket-icons'
import { BUCKET_COLORS, isHexColor } from './bucket'

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
  const current = value ?? DEFAULT_BUCKET_ICON
  const custom = value !== undefined && !BUCKET_ICONS.includes(value)
  return (
    <div className="storage-cfg-icons" role="radiogroup" aria-label={label}>
      {custom ? (
        // Written by another client: kept as stored, shown as the fallback glyph.
        // biome-ignore lint/a11y/useSemanticElements: icon buttons styled as a radio group; native radios would need the whole control restyled
        <button
          type="button"
          role="radio"
          aria-checked
          className="storage-cfg-icon"
          title={`${value} (kept as stored)`}
        >
          <BucketIcon name={value} color={color} />
        </button>
      ) : null}
      {BUCKET_ICONS.map((icon) => {
        const selected = !custom && icon === current
        return (
          // biome-ignore lint/a11y/useSemanticElements: icon buttons styled as a radio group; native radios would need the whole control restyled
          <button
            key={icon}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={icon}
            title={icon}
            className="storage-cfg-icon"
            onClick={() => onChange(icon === DEFAULT_BUCKET_ICON ? undefined : icon)}
          >
            <BucketIcon name={icon} color={selected ? color : undefined} />
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
  const custom = normalized !== undefined && !BUCKET_COLORS.includes(normalized)
  return (
    <div className="storage-cfg-swatches" role="radiogroup" aria-label={label}>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        aria-pressed={value === undefined}
        onClick={() => onChange(undefined)}
      >
        None
      </Button>
      {BUCKET_COLORS.map((color) => (
        // biome-ignore lint/a11y/useSemanticElements: colour swatches styled as a radio group; native radios would need the whole control restyled
        <button
          key={color}
          type="button"
          role="radio"
          aria-checked={normalized === color}
          aria-label={color}
          title={color}
          className="storage-cfg-swatch"
          style={{ background: color }}
          onClick={() => onChange(color)}
        />
      ))}
      <label className="storage-cfg-swatch-custom" title="Custom colour">
        <input
          type="color"
          aria-label="Custom colour"
          value={isHexColor(value) ? value : '#22C55E'}
          onChange={(event) => onChange(event.target.value.toUpperCase())}
        />
        {custom ? <span className="storage-cfg-mono">{value}</span> : <span>Custom</span>}
      </label>
    </div>
  )
}
