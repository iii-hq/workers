import { Button, CodeEditor, StatusPanel } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useState } from 'react'
import type { Api } from './api'
import { SaveBar } from './ConnectionView'
import { mapAsJson, readMapEdit } from './config-model'
import { removeEntry, type SaveDeps, saveEntry, secretIssues } from './save'
import type { Issue } from './types'

/** Edit as JSON: every entry in one editor, saved entry by entry through the same sequence as the form. A
 * value in a secret position shows hidden; an entry left as shown is no change. */
export function JsonView({
  api,
  deps,
  mcpServers,
  onClose,
  setDirty,
  onMoveSecrets,
}: {
  api: Api
  deps: SaveDeps
  mcpServers: Record<string, unknown>
  onClose: () => void
  setDirty: (dirty: boolean) => void
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
      const { servers } = await api.validate(read.candidate)
      const errors = Object.values(servers).flatMap((v) => v.errors)
      setIssues(errors)
      if (errors.length > 0) return
      const listed = (await api.listServers()).servers
      const notes: Issue[] = []
      // Name order across both kinds; the first declined or failed one stops the rest.
      for (const name of [...read.removed, ...read.changed].sort()) {
        const server = listed.find((s) => s.name === name)
        if (read.removed.includes(name)) {
          if (!server) notes.push({ path: '', message: `${name}: not removed, the worker does not list it` })
          else if (!(await removeEntry(deps, server))) return
          continue
        }
        const before = server && name in shown ? { entry: shown[name], fingerprint: server.fingerprint } : null
        const result = await saveEntry(deps, name, read.candidate[name] as Record<string, unknown>, before)
        if (!result.saved) {
          setIssues(result.errors)
          setFailure(result.message ?? null)
          return
        }
      }
      if (notes.length > 0) return setIssues(notes)
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
        <SaveBar onSave={() => void save()} onDiscard={discard} saving={saving} blocked={null} />
      ) : null}
    </div>
  )
}
