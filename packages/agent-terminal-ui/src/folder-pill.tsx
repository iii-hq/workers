/**
 * The pane's own folder: the console's working-directory picker
 * (`DirectoryPicker`, the same one the chat composer uses: remembered
 * projects first, a level-by-level browse over `shell::workspace::*`, every
 * pick validated against the live shell), set in the terminal's bottom row.
 *
 * It is this pane's folder, not the chat's: picking one restarts the agent
 * there and leaves the chat where it is.
 */
import { DirectoryPicker } from '@iii-dev/console-ui'

export interface FolderPillProps {
  /** The folder the session runs in; null until the worker has named it. */
  value: string | null
  /** The compose project's folder, pinned at the top of the picker. */
  projectDir?: string | null
  /** While a session is starting: a second start would race the first. */
  disabled: boolean
  onChange: (dir: string) => void
}

export function FolderPill({ value, projectDir, disabled, onChange }: FolderPillProps) {
  return (
    <span className="agent-terminal-folder">
      <DirectoryPicker
        value={value}
        defaultDir={projectDir ?? null}
        disabled={disabled}
        emptyLabel="Project folder"
        onChange={(dir) => {
          if (dir !== value) onChange(dir)
        }}
      />
    </span>
  )
}
