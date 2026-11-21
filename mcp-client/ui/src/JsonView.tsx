import { Button, CodeEditor, StatusPanel } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useState } from 'react'
import { SaveBar } from './ConnectionView'
import { mapAsJson, readMapEdit } from './config-model'
import { type SaveDeps, saveMap, secretIssues } from './save'
import type { Issue } from './types'

/** Edit as JSON: every entry in one editor, saved entry by entry through the same sequence as the form. A
 * value in a secret position shows hidden; an entry left as shown is no change. `blocked` says why Save cannot
 * run. */
export function JsonView({
  deps,
  mcpServers,
  onClose,
  setDirty,
  blocked,
  onMoveSecrets,
}: {
  deps: SaveDeps
  mcpServers: Record<string, unknown>
  onClose: () => void
  setDirty: (dirty: boolean) => void
  blocked: string | null
  onMoveSecrets?: (text: string) => void
}) {
  // The map the text showed: an edit is read against it, not against a refresh that came later.
  const [shown] = useState(mcpServers)
  const [first] = useState(() => mapAsJson(mcpServers))
  const [text, setText] = useState(first)
  const [issues, setIssues] = useState<Issue[]>([])
  const [failure, setFailure] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const dirty = text !== first

  useEffect(() => setDirty(dirty), [dirty, setDirty])
  useEffect(() => () => setDirty(false), [setDirty])

  const save = async () => {
    setFailure(null)
    const read = readMapEdit(shown, text)
    if ('issues' in read) return setIssues(read.issues)
    setSaving(true)
    try {
      const result = await saveMap(deps, shown, read, (await deps.api.listServers()).servers)
      setIssues(result.issues)
      setFailure(result.message ?? null)
      if (!result.saved || result.issues.length > 0) return
      setDirty(false)
      onClose()
    } catch (err) {
      setFailure(errorMessage(err))
    } finally {
      setSaving(false)
    }
  }
  const discard = () => {
    setText(first)
    setIssues([])
    setFailure(null)
  }

  return (
    <div className="mcp-ui-json-edit">
      <CodeEditor
        className="mcp-ui-payload-json"
        language="json"
        fill
        lineNumbers
        value={text}
        onChange={setText}
        aria-label="MCP servers as JSON"
      />
      {failure ? <StatusPanel variant="alert" role="alert" headline="Could not save" detail={failure} /> : null}
      {onMoveSecrets && secretIssues(issues).length > 0 ? (
        <Button variant="secondary" size="sm" onClick={() => onMoveSecrets(text)}>
          Move secrets to the vault
        </Button>
      ) : null}
      {issues.length > 0 ? (
        <ul className="mcp-ui-issues" role="alert">
          {issues.map((issue, i) => (
            <li key={i}>
              {issue.path ? <code>{issue.path}</code> : null}
              <span>{issue.message}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {dirty || saving ? (
        <SaveBar onSave={() => void save()} onDiscard={discard} saving={saving} blocked={blocked} />
      ) : null}
    </div>
  )
}
