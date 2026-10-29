/* The Editor section at the top of the IDE's settings. Unlike every other
   field on the page these are not part of the worker's configuration entry:
   text sizes and what the editor header shows belong to the person at this
   screen, so they are kept in this browser (the same stores the IDE page
   reads) and apply as soon as they change, with no Save. */

import {
  DEFAULT_EDITOR_FONT_SIZE,
  EDITOR_FONT_FAMILIES,
  MAX_EDITOR_FONT_SIZE,
  MIN_EDITOR_FONT_SIZE,
  useEditorPrefs,
} from '@iii-workers/editor-prefs'
import { DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MIN_FONT_SIZE, useTerminalFontSize } from '@iii-workers/terminal-font'
import { Select, SettingsField, SettingsList, SettingsSection, Switch } from '@iii-dev/console-ui'

const SAVE_KEY = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘S' : 'Ctrl+S'

/** One option per pixel, keeping the current size even when it sits outside the range. */
function sizeOptions(min: number, max: number, current: number, fallback: number) {
  const sizes = new Set<number>([current])
  for (let size = min; size <= max; size += 1) sizes.add(size)
  return [...sizes]
    .sort((a, b) => a - b)
    .map((size) => ({ value: String(size), label: size === fallback ? `${size} px (default)` : `${size} px` }))
}

export function IdeEditorSection() {
  const [prefs, setPrefs] = useEditorPrefs()
  const [terminalSize, setTerminalSize] = useTerminalFontSize()
  return (
    <SettingsSection title="Editor" description="Kept in this browser and applied as soon as you change them.">
      <SettingsList>
        <SettingsField
          label="Code font"
          description="The face for code in the editor, diffs and the terminal. A font not installed on this machine falls back to Geist Mono."
          controlSize="default"
          renderControl={(c) => (
            <Select
              id={c.id}
              value={prefs.fontFamily}
              onChange={(next) => setPrefs({ fontFamily: next })}
              options={EDITOR_FONT_FAMILIES.map((family) => ({ value: family.id, label: family.label }))}
            />
          )}
        />
        <SettingsField
          label="Editor text size"
          description="Code in the editor, diffs and older versions of a file."
          controlSize="compact"
          renderControl={(c) => (
            <Select
              id={c.id}
              value={String(prefs.fontSize)}
              onChange={(next) => setPrefs({ fontSize: Number(next) })}
              options={sizeOptions(MIN_EDITOR_FONT_SIZE, MAX_EDITOR_FONT_SIZE, prefs.fontSize, DEFAULT_EDITOR_FONT_SIZE)}
            />
          )}
        />
        <SettingsField
          label="Terminal text size"
          description="Every terminal in the console follows this size."
          controlSize="compact"
          renderControl={(c) => (
            <Select
              id={c.id}
              value={String(terminalSize)}
              onChange={(next) => setTerminalSize(Number(next))}
              options={sizeOptions(Math.max(MIN_FONT_SIZE, 10), Math.min(MAX_FONT_SIZE, 24), terminalSize, DEFAULT_FONT_SIZE)}
            />
          )}
        />
        <SettingsField
          label="Show file size and line count"
          description="Beside the file path above the editor. The line count opens Go to line."
          controlSize="fit"
          renderControl={(c) => (
            <Switch
              id={c.id}
              checked={prefs.showFileInfo}
              onChange={(event) => setPrefs({ showFileInfo: event.currentTarget.checked })}
            />
          )}
        />
        <SettingsField
          label="Show a Save button"
          description={`At the end of the editor header. ${SAVE_KEY} saves either way.`}
          controlSize="fit"
          renderControl={(c) => (
            <Switch
              id={c.id}
              checked={prefs.showSaveButton}
              onChange={(event) => setPrefs({ showSaveButton: event.currentTarget.checked })}
            />
          )}
        />
      </SettingsList>
    </SettingsSection>
  )
}
