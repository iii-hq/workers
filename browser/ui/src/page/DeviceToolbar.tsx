/**
 * The device toolbar: pin the session's viewport to a device size the way a
 * browser's responsive/device mode does. A preset or a custom width x height
 * sets the viewport (through `browser::resize`); while a device is pinned the
 * viewport no longer tracks the pane. Reset returns to tracking the pane.
 */

import { Button, Input, Selector } from '@iii-dev/console-ui'
import { RefreshCw } from '../lib/icons'

export interface DevicePreset {
  id: string
  label: string
  width: number
  height: number
  deviceScaleFactor: number
  mobile: boolean
}

export const DEVICE_PRESETS: readonly DevicePreset[] = [
  {
    id: 'mobile-s',
    label: 'Mobile S · 320',
    width: 320,
    height: 568,
    deviceScaleFactor: 2,
    mobile: true,
  },
  {
    id: 'mobile-m',
    label: 'iPhone · 375',
    width: 375,
    height: 812,
    deviceScaleFactor: 3,
    mobile: true,
  },
  {
    id: 'mobile-l',
    label: 'Mobile L · 430',
    width: 430,
    height: 932,
    deviceScaleFactor: 3,
    mobile: true,
  },
  {
    id: 'tablet',
    label: 'iPad · 768',
    width: 768,
    height: 1024,
    deviceScaleFactor: 2,
    mobile: true,
  },
  {
    id: 'laptop',
    label: 'Laptop · 1280',
    width: 1280,
    height: 800,
    deviceScaleFactor: 1,
    mobile: false,
  },
  {
    id: 'desktop',
    label: 'Desktop · 1440',
    width: 1440,
    height: 900,
    deviceScaleFactor: 1,
    mobile: false,
  },
]

export interface DeviceState {
  width: number
  height: number
  deviceScaleFactor: number
  mobile: boolean
  presetId: string | null
}

interface DeviceToolbarProps {
  device: DeviceState
  onPreset: (preset: DevicePreset) => void
  onDimensions: (width: number, height: number) => void
  onRotate: () => void
  onReset: () => void
}

function parseDimension(value: string): number | null {
  const n = Number(value)
  return Number.isFinite(n) ? n : null
}

export function DeviceToolbar({
  device,
  onPreset,
  onDimensions,
  onRotate,
  onReset,
}: DeviceToolbarProps) {
  return (
    <fieldset className="br-ui-device" aria-label="device toolbar">
      <legend className="br-ui-visually-hidden">device toolbar</legend>
      <Selector
        value={device.presetId ?? 'custom'}
        onChange={(id) => {
          const preset = DEVICE_PRESETS.find((p) => p.id === id)
          if (preset) onPreset(preset)
        }}
        options={[
          { value: 'custom', label: 'Custom' },
          ...DEVICE_PRESETS.map((p) => ({ value: p.id, label: p.label })),
        ]}
        aria-label="device preset"
        className="br-ui-device-select"
      />
      <span className="br-ui-device-dims">
        <Input
          value={String(device.width)}
          onChange={(next) => {
            const w = parseDimension(next)
            if (w !== null) onDimensions(w, device.height)
          }}
          aria-label="viewport width"
          inputMode="numeric"
          className="br-ui-device-num"
        />
        <span aria-hidden className="br-ui-device-x">
          ×
        </span>
        <Input
          value={String(device.height)}
          onChange={(next) => {
            const h = parseDimension(next)
            if (h !== null) onDimensions(device.width, h)
          }}
          aria-label="viewport height"
          inputMode="numeric"
          className="br-ui-device-num"
        />
      </span>
      <span className="br-ui-device-dpr">{device.deviceScaleFactor}×</span>
      <Button
        variant="ghost"
        size="sm"
        onClick={onRotate}
        title="rotate"
        aria-label="rotate viewport"
      >
        <RefreshCw size={16} aria-hidden />
      </Button>
      <Button variant="ghost" size="sm" onClick={onReset} title="fit to pane">
        Reset
      </Button>
    </fieldset>
  )
}
