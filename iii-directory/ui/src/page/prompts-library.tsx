/**
 * The prompts tab's library surface: every prompt the directory worker
 * serves — worker-shipped slash templates and user library entries, in
 * `command` and `system` kinds — in a left rail, the selected prompt's
 * fields + body in an editor on the right. User library entries save,
 * fork (save under a new name), and delete in place through
 * `directory::prompts::save` / `delete`; worker-shipped templates render
 * read-only (they ship with their worker's bundle).
 *
 * Unlike the skills tab's raw-markdown editor, prompts edit as structured
 * fields (name, kind, description, body) because the worker reconstructs
 * the frontmatter on save — the same shape the composer's slash picker and
 * the send/spawn system-prompt options consume.
 */

import { Badge, Button, CodeEditor, type Host, Input } from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import { formatRelativeTime } from '../lib/format'
import { useOnChange } from './browser'

interface PromptRow {
  name: string
  description: string
  kind: string
  source: string
  modified_at: string
}

interface PromptDetail extends PromptRow {
  body: string
}

interface Draft {
  name: string
  description: string
  kind: string
  body: string
  /** Existing user-library entry (save overwrites) vs a new/forked one. */
  existing: boolean
  /** Worker-shipped rows render read-only. */
  readonly: boolean
}

const EMPTY_DRAFT: Draft = {
  name: '',
  description: '',
  kind: 'system',
  body: '',
  existing: false,
  readonly: false,
}

export function PromptsLibrary({ host }: { host: Host }) {
  const [rows, setRows] = useState<PromptRow[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [search, setSearch] = useState('')
  const [draft, setDraft] = useState<Draft | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [savedFlash, setSavedFlash] = useState(false)

  const refresh = useCallback(() => {
    host.iii
      .trigger<{ prompts: PromptRow[] }>('directory::prompts::list', {})
      .then((out) => {
        setRows(out.prompts ?? [])
        setListError(null)
      })
      .catch((e) => setListError(String(e)))
  }, [host])

  useEffect(() => {
    refresh()
  }, [refresh])

  // A save/delete/download landing anywhere re-reads the store.
  useOnChange(host, 'directory::prompts::on-change', refresh)

  const open = useCallback(
    (row: PromptRow) => {
      setError(null)
      host.iii
        .trigger<PromptDetail>('directory::prompts::get', { name: row.name })
        .then((detail) => {
          setDraft({
            name: detail.name,
            description: detail.description ?? '',
            kind: detail.kind ?? 'command',
            body: detail.body ?? '',
            existing: detail.source === 'user',
            readonly: detail.source !== 'user',
          })
        })
        .catch((e) => setError(String(e)))
    },
    [host],
  )

  const act = useCallback(
    async (fn: () => Promise<unknown>) => {
      setBusy(true)
      setError(null)
      try {
        await fn()
        refresh()
        return true
      } catch (e) {
        setError(String(e))
        return false
      } finally {
        setBusy(false)
      }
    },
    [refresh],
  )

  const onSave = useCallback(async () => {
    if (!draft) return
    const ok = await act(() =>
      host.iii.trigger('directory::prompts::save', {
        name: draft.name.trim(),
        description: draft.description.trim(),
        body: draft.body,
        kind: draft.kind,
      }),
    )
    if (ok) {
      setDraft({ ...draft, existing: true })
      setSavedFlash(true)
      window.setTimeout(() => setSavedFlash(false), 1600)
    }
  }, [act, draft, host])

  const onFork = useCallback(() => {
    if (!draft) return
    setDraft({
      ...draft,
      name: `${draft.name}-fork`,
      existing: false,
      readonly: false,
    })
  }, [draft])

  const onDelete = useCallback(async () => {
    if (!draft) return
    if (
      !window.confirm(`Delete prompt "${draft.name}"? This cannot be undone.`)
    )
      return
    const ok = await act(() =>
      host.iii.trigger('directory::prompts::delete', {
        name: draft.name,
        yes: true,
      }),
    )
    if (ok) setDraft(null)
  }, [act, draft, host])

  const needle = search.trim().toLowerCase()
  const visible = (rows ?? []).filter(
    (r) =>
      !needle ||
      r.name.toLowerCase().includes(needle) ||
      r.description.toLowerCase().includes(needle),
  )

  const canSave =
    draft !== null &&
    !draft.readonly &&
    !busy &&
    draft.name.trim().length > 0 &&
    draft.description.trim().length > 0

  return (
    <div className="dir-ui-browser">
      <div className="dir-ui-side">
        <div className="dir-ui-side-search dir-ui-prompt-side-head">
          <Input
            value={search}
            onChange={setSearch}
            placeholder="filter prompts…"
            aria-label="filter prompts"
          />
          <Button
            variant="primary"
            size="sm"
            disabled={busy}
            onClick={() => {
              setError(null)
              setDraft({ ...EMPTY_DRAFT })
            }}
          >
            new
          </Button>
        </div>
        {listError ? (
          <div className="dir-ui-error">{listError}</div>
        ) : rows === null ? (
          <div className="dir-ui-pulse">· loading prompts…</div>
        ) : visible.length === 0 ? (
          <div className="dir-ui-empty">
            · no prompts{needle ? ' match' : ' yet'}
          </div>
        ) : (
          <ul className="dir-ui-side-list">
            {visible.map((r) => (
              <li key={`${r.source}:${r.name}`}>
                <button
                  type="button"
                  className={`dir-ui-side-row${
                    draft?.name === r.name ? ' active' : ''
                  }`}
                  onClick={() => open(r)}
                >
                  <span className="dir-ui-prompt-row-head">
                    <span className="dir-ui-id">{r.name}</span>
                    <Badge variant={r.kind === 'system' ? 'accent' : 'default'}>
                      {r.kind}
                    </Badge>
                    {r.source === 'user' ? <Badge>user</Badge> : null}
                  </span>
                  {r.description ? (
                    <span className="dir-ui-desc clamp">{r.description}</span>
                  ) : null}
                  <span className="dir-ui-fine">
                    {formatRelativeTime(r.modified_at)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="dir-ui-editor">
        {!draft ? (
          <div className="dir-ui-empty pad">
            · select a prompt to view or edit it, or create a new one
          </div>
        ) : (
          <>
            <div className="dir-ui-editor-head dir-ui-prompt-fields">
              <Input
                value={draft.name}
                onChange={(v) => setDraft({ ...draft, name: v })}
                placeholder="prompt-name"
                aria-label="prompt name"
                disabled={draft.existing || draft.readonly}
              />
              <select
                className="dir-ui-input dir-ui-prompt-kind"
                value={draft.kind}
                onChange={(e) => setDraft({ ...draft, kind: e.target.value })}
                disabled={draft.readonly}
                aria-label="prompt kind"
              >
                <option value="system">system</option>
                <option value="command">command</option>
              </select>
              {draft.readonly ? (
                <Badge variant="warn">worker-shipped · read-only</Badge>
              ) : null}
              <span className="dir-ui-prompt-spacer" />
              {!draft.readonly ? (
                <Button
                  variant="primary"
                  size="sm"
                  disabled={!canSave}
                  onClick={() => void onSave()}
                >
                  {busy ? 'saving…' : 'save'}
                </Button>
              ) : null}
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={onFork}
                title="edit a copy under a new name"
              >
                fork
              </Button>
              {draft.existing && !draft.readonly ? (
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy}
                  onClick={() => void onDelete()}
                >
                  delete
                </Button>
              ) : null}
              <span className="dir-ui-save-note" aria-live="polite">
                {savedFlash ? 'saved' : ''}
              </span>
            </div>
            <div className="dir-ui-prompt-desc">
              <Input
                value={draft.description}
                onChange={(v) => setDraft({ ...draft, description: v })}
                placeholder="one-line description (shown in the list and pickers)"
                aria-label="prompt description"
                disabled={draft.readonly}
              />
            </div>
            {error ? <div className="dir-ui-error">{error}</div> : null}
            <div className="dir-ui-editor-body mode-edit dir-ui-prompt-body">
              <div className="dir-ui-pane">
                <CodeEditor
                  value={draft.body}
                  onChange={(v) => setDraft({ ...draft, body: v })}
                  language="markdown"
                  className="dir-ui-code"
                  aria-label="prompt body"
                  placeholder="the prompt text…"
                  readOnly={draft.readonly}
                />
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
