/**
 * The export cluster both panes share: background on/off, light/dark, then
 * svg + png. Options are pane-local state seeded from the console theme —
 * exporting a dark png out of a light console is one click, not a theme
 * switch.
 */

import { Button } from '@iii-dev/console-ui'
import { useState } from 'react'
import { Download } from './icons'

export interface ExportOptions {
  format: 'svg' | 'png'
  dark: boolean
  /** Paint the theme background; off = transparent. */
  background: boolean
}

interface ExportMenuProps {
  theme: 'light' | 'dark'
  disabled: boolean
  onExport: (opts: ExportOptions) => void
}

export function ExportMenu({ theme, disabled, onExport }: ExportMenuProps) {
  const [dark, setDark] = useState(theme === 'dark')
  const [background, setBackground] = useState(true)

  return (
    <span className="cv-export">
      <button
        type="button"
        className={`cv-export-opt${background ? ' on' : ''}`}
        onClick={() => setBackground((v) => !v)}
        aria-pressed={background}
        title={
          background
            ? 'exports paint the theme background — click for transparent'
            : 'exports are transparent — click to paint the background'
        }
      >
        bg
      </button>
      <button
        type="button"
        className={`cv-export-opt${dark ? ' on' : ''}`}
        onClick={() => setDark((v) => !v)}
        aria-pressed={dark}
        title="theme the export dark or light, independent of the console"
      >
        {dark ? 'dark' : 'light'}
      </button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onExport({ format: 'svg', dark, background })}
        disabled={disabled}
        title="download as svg"
      >
        <Download size={13} aria-hidden />
        svg
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => onExport({ format: 'png', dark, background })}
        disabled={disabled}
        title="download as png"
      >
        <Download size={13} aria-hidden />
        png
      </Button>
    </span>
  )
}
