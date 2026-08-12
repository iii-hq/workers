/**
 * The files tab: a lazy directory tree (`sandbox::fs::ls` per expand, root
 * `/`), a grep toolbar (`sandbox::fs::grep` from `/`, matches jump to the
 * file), small mkdir/rm/mv/chmod op dialogs, and the viewer/editor pane —
 * `sandbox::fs::read` bodies render inline; a StreamChannelRef (or absent
 * body) renders as a size card with a base64 download escape hatch
 * (`sandbox::exec sh -c 'base64 <path>'`, decoded client-side — the exec
 * inline cap means ~1 MiB is the honest ceiling). Editing goes back
 * through `sandbox::fs::write { content }`.
 *
 * A stopped sandbox has NO filesystem anymore — the whole tab collapses to
 * the warn banner.
 */

import {
  Button,
  CodeEditor,
  Dialog,
  DialogContent,
  DialogTitle,
  type Host,
  Input,
} from '@iii-dev/console-ui'
import { type ReactNode, useCallback, useEffect, useState } from 'react'
import { asRecord } from '../lib/payload'
import { decodeBase64, parseExecResult, quoteShellArg } from './exec'
import { basename, formatBytes } from './format'
import { ChevronIcon, DownloadIcon, FileIcon, FolderIcon } from './icons'
import type { SandboxSummary } from './store'

interface FsEntry {
  name: string
  is_dir: boolean
  size: number
  mode: string
  is_symlink: boolean
}

interface GrepMatch {
  path: string
  line: number
  content: string
}

type FileView =
  | { kind: 'text'; path: string; content: string; size: number }
  | { kind: 'binary'; path: string; size: number }
  | { kind: 'error'; path: string; message: string }

function parseEntries(value: unknown): FsEntry[] {
  const list = asRecord(value)?.entries
  if (!Array.isArray(list)) return []
  const out: FsEntry[] = []
  for (const item of list) {
    const rec = asRecord(item)
    if (!rec || typeof rec.name !== 'string') continue
    out.push({
      name: rec.name,
      is_dir: rec.is_dir === true,
      size: typeof rec.size === 'number' ? rec.size : 0,
      mode: typeof rec.mode === 'string' ? rec.mode : '',
      is_symlink: rec.is_symlink === true,
    })
  }
  // Directories first, then by name.
  return out.sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1
    return a.name.localeCompare(b.name)
  })
}

function parseMatches(value: unknown): GrepMatch[] {
  const list = asRecord(value)?.matches
  if (!Array.isArray(list)) return []
  const out: GrepMatch[] = []
  for (const item of list) {
    const rec = asRecord(item)
    if (!rec) continue
    // `path` is canonical; older guests sent `file`.
    const path = [rec.path, rec.file].find(
      (v): v is string => typeof v === 'string',
    )
    if (!path) continue
    out.push({
      path,
      line: typeof rec.line === 'number' ? rec.line : 0,
      content: typeof rec.content === 'string' ? rec.content : '',
    })
  }
  return out
}

function joinPath(dir: string, name: string): string {
  return dir === '/' ? `/${name}` : `${dir}/${name}`
}

/** Immediate parent directory of an absolute path: `/a/b` → `/a`, `/x` → `/`. */
function parentDir(path: string): string {
  const clean = path.replace(/\/+$/, '')
  const idx = clean.lastIndexOf('/')
  return idx <= 0 ? '/' : clean.slice(0, idx)
}

type OpKind = 'mkdir' | 'rm' | 'mv' | 'chmod'

const OP_FIELDS: Record<OpKind, { a: string; b: string | null }> = {
  mkdir: { a: 'path', b: null },
  rm: { a: 'path', b: null },
  mv: { a: 'src', b: 'dst' },
  chmod: { a: 'path', b: 'mode (e.g. 0755)' },
}

export function FilesTab({
  host,
  sandbox,
}: {
  host: Host
  sandbox: SandboxSummary
}) {
  const sandboxId = sandbox.sandbox_id
  const [dirs, setDirs] = useState<Record<string, FsEntry[]>>({})
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(['/']))
  const [treeError, setTreeError] = useState<string | null>(null)
  const [file, setFile] = useState<FileView | null>(null)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [saving, setSaving] = useState(false)
  const [downloading, setDownloading] = useState(false)
  const [pattern, setPattern] = useState('')
  const [matches, setMatches] = useState<GrepMatch[] | null>(null)
  const [searching, setSearching] = useState(false)
  const [op, setOp] = useState<OpKind | null>(null)
  const [opA, setOpA] = useState('')
  const [opB, setOpB] = useState('')
  const [opRecursive, setOpRecursive] = useState(false)
  const [opBusy, setOpBusy] = useState(false)
  const [opError, setOpError] = useState<string | null>(null)

  const loadDir = useCallback(
    (path: string) => {
      host.iii
        .trigger('sandbox::fs::ls', { sandbox_id: sandboxId, path })
        .then((value) => {
          setDirs((prev) => ({ ...prev, [path]: parseEntries(value) }))
          setTreeError(null)
        })
        .catch((err: unknown) =>
          setTreeError(err instanceof Error ? err.message : String(err)),
        )
    },
    [host, sandboxId],
  )

  useEffect(() => {
    if (!sandbox.stopped) loadDir('/')
  }, [loadDir, sandbox.stopped])

  const toggleDir = (path: string) => {
    // The load side effect stays outside the updater — React may invoke
    // updaters more than once.
    const opening = !expanded.has(path)
    setExpanded((prev) => {
      const next = new Set(prev)
      if (opening) next.add(path)
      else next.delete(path)
      return next
    })
    if (opening && !(path in dirs)) loadDir(path)
  }

  const openFile = useCallback(
    (path: string) => {
      setEditing(false)
      setFile(null)
      host.iii
        .trigger('sandbox::fs::read', { sandbox_id: sandboxId, path })
        .then((value) => {
          const rec = asRecord(value)
          const size = typeof rec?.size === 'number' ? rec.size : 0
          // Inline UTF-8 body, or a StreamChannelRef object / nothing —
          // anything non-string is "no inline body" and gets the size card.
          if (typeof rec?.content === 'string') {
            setFile({ kind: 'text', path, content: rec.content, size })
          } else {
            setFile({ kind: 'binary', path, size })
          }
        })
        .catch((err: unknown) =>
          setFile({
            kind: 'error',
            path,
            message: err instanceof Error ? err.message : String(err),
          }),
        )
    },
    [host, sandboxId],
  )

  const refreshAfterOp = (touched: string[]) => {
    // Reload exactly the listings the op changed: each touched path's
    // parent (when that dir is loaded) plus the root — once, the Set
    // dedupes a touched parent that IS the root.
    const parents = new Set(touched.map(parentDir))
    parents.add('/')
    for (const dir of parents) {
      if (dir === '/' || dir in dirs) loadDir(dir)
    }
  }

  const runOp = () => {
    if (!op) return
    const payloads: Record<OpKind, Record<string, unknown>> = {
      mkdir: { sandbox_id: sandboxId, path: opA, parents: true },
      rm: { sandbox_id: sandboxId, path: opA, recursive: opRecursive },
      mv: { sandbox_id: sandboxId, src: opA, dst: opB },
      chmod: { sandbox_id: sandboxId, path: opA, mode: opB },
    }
    setOpBusy(true)
    setOpError(null)
    host.iii
      .trigger(`sandbox::fs::${op}`, payloads[op])
      .then(() => {
        setOp(null)
        setOpA('')
        setOpB('')
        setOpRecursive(false)
        refreshAfterOp([opA, opB].filter(Boolean))
      })
      .catch((err: unknown) =>
        setOpError(err instanceof Error ? err.message : String(err)),
      )
      .finally(() => setOpBusy(false))
  }

  const grep = () => {
    if (!pattern.trim()) return
    setSearching(true)
    setMatches(null)
    host.iii
      .trigger('sandbox::fs::grep', {
        sandbox_id: sandboxId,
        path: '/',
        pattern,
        recursive: true,
        max_matches: 200,
      })
      .then((value) => setMatches(parseMatches(value)))
      .catch((err: unknown) =>
        setMatches([
          {
            path: '',
            line: 0,
            content: `grep failed: ${err instanceof Error ? err.message : String(err)}`,
          },
        ]),
      )
      .finally(() => setSearching(false))
  }

  const save = () => {
    if (!file || file.kind !== 'text') return
    setSaving(true)
    host.iii
      .trigger('sandbox::fs::write', {
        sandbox_id: sandboxId,
        path: file.path,
        content: draft,
      })
      .then(() => {
        setEditing(false)
        openFile(file.path)
      })
      .catch((err: unknown) =>
        setFile({
          kind: 'error',
          path: file.path,
          message: err instanceof Error ? err.message : String(err),
        }),
      )
      .finally(() => setSaving(false))
  }

  const download = (path: string) => {
    setDownloading(true)
    host.iii
      .trigger(
        'sandbox::exec',
        {
          sandbox_id: sandboxId,
          argv: ['sh', '-c', `base64 ${quoteShellArg(path)}`],
          timeout_ms: 30_000,
        },
        { timeoutMs: 60_000 },
      )
      .then((value) => {
        const result = parseExecResult(value)
        if (result.exit_code !== 0) {
          throw new Error(result.stderr || `base64 exited ${result.exit_code}`)
        }
        const bytes = decodeBase64(result.stdout)
        const blob = new Blob([bytes])
        const url = URL.createObjectURL(blob)
        const a = document.createElement('a')
        a.href = url
        a.download = basename(path)
        a.click()
        // Deferred: a synchronous revoke can abort the download mid-flight.
        window.setTimeout(() => URL.revokeObjectURL(url), 0)
      })
      .catch((err: unknown) =>
        setFile({
          kind: 'error',
          path,
          message: err instanceof Error ? err.message : String(err),
        }),
      )
      .finally(() => setDownloading(false))
  }

  if (sandbox.stopped) {
    return (
      <div className="cr-page-files">
        <div className="cr-page-banner warn" role="status">
          sandbox ended — filesystem gone. The microVM's disk was destroyed
          with it; nothing here can be browsed or restored.
        </div>
      </div>
    )
  }

  const renderDir = (path: string, depth: number) => {
    const entries = dirs[path]
    if (!entries) {
      return (
        <div className="cr-page-faint cr-page-tree-loading" style={{ paddingLeft: depth * 14 + 8 }}>
          loading…
        </div>
      )
    }
    if (entries.length === 0) {
      return (
        <div className="cr-page-faint cr-page-tree-loading" style={{ paddingLeft: depth * 14 + 8 }}>
          empty
        </div>
      )
    }
    return entries.map((entry) => {
      const full = joinPath(path, entry.name)
      if (entry.is_dir) {
        const open = expanded.has(full)
        return (
          <div key={full}>
            <button
              type="button"
              className="cr-page-tree-row"
              style={{ paddingLeft: depth * 14 + 8 }}
              onClick={() => toggleDir(full)}
              aria-expanded={open}
            >
              <ChevronIcon className={open ? 'cr-page-chev open' : 'cr-page-chev'} aria-hidden />
              <FolderIcon aria-hidden />
              <span className="cr-page-tree-name">{entry.name}</span>
            </button>
            {open ? renderDir(full, depth + 1) : null}
          </div>
        )
      }
      return (
        <button
          key={full}
          type="button"
          className={`cr-page-tree-row${file?.path === full ? ' selected' : ''}`}
          style={{ paddingLeft: depth * 14 + 22 }}
          onClick={() => openFile(full)}
        >
          <FileIcon aria-hidden />
          <span className="cr-page-tree-name">{entry.name}</span>
          <span className="cr-page-tree-size">{formatBytes(entry.size)}</span>
        </button>
      )
    })
  }

  function renderViewer(): ReactNode {
    if (file === null) {
      // The grep results already fill the pane when there are any.
      if (matches !== null) return null
      return (
        <div className="cr-page-faint cr-page-viewer-empty">
          pick a file on the left, or grep for one.
        </div>
      )
    }
    if (file.kind === 'error') {
      return (
        <div className="cr-page-errcard" role="alert">
          <div className="cr-page-errcard-head">
            <code className="cr-page-errcode">{file.path}</code>
          </div>
          <div className="cr-page-errcard-msg">{file.message}</div>
        </div>
      )
    }
    if (file.kind === 'binary') {
      return (
        <div className="cr-page-bincard">
          <div className="cr-page-bincard-title">
            <code>{file.path}</code>
          </div>
          <div className="cr-page-bincard-size">
            {formatBytes(file.size)} — no inline body (binary or past the
            inline cap)
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => download(file.path)}
            disabled={downloading}
            title="sandbox::exec sh -c 'base64 <path>' → decoded client-side. Exec output inlines up to 1 MiB, so larger files come back truncated."
          >
            <DownloadIcon aria-hidden />{' '}
            {downloading ? 'downloading…' : 'download via base64 (≤ 1 MiB)'}
          </Button>
        </div>
      )
    }
    return (
      <div className="cr-page-filepane">
        <div className="cr-page-filepane-head">
          <code className="cr-page-filepane-path" title={file.path}>
            {file.path}
          </code>
          <span className="cr-page-faint">{formatBytes(file.size)}</span>
          <span className="cr-page-actions-gap" />
          {editing ? (
            <>
              <Button variant="ghost" size="sm" onClick={() => setEditing(false)} disabled={saving}>
                cancel
              </Button>
              <Button variant="primary" size="sm" onClick={save} disabled={saving}>
                {saving ? 'saving…' : 'save'}
              </Button>
            </>
          ) : (
            <>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setDraft(file.content)
                  setEditing(true)
                }}
              >
                edit
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => download(file.path)}
                disabled={downloading}
                title="download a byte-exact copy via base64 (≤ 1 MiB)"
              >
                <DownloadIcon aria-hidden />
              </Button>
            </>
          )}
        </div>
        {editing ? (
          <CodeEditor
            value={draft}
            onChange={setDraft}
            language="plaintext"
            className="cr-page-file-editor"
            aria-label={`edit ${file.path}`}
          />
        ) : (
          <pre className="cr-page-file-body">{file.content}</pre>
        )}
      </div>
    )
  }

  const opField = op ? OP_FIELDS[op] : null

  return (
    <div className="cr-page-files">
      <div className="cr-page-files-toolbar">
        <Input
          value={pattern}
          onChange={setPattern}
          onKeyDown={(e) => e.key === 'Enter' && grep()}
          placeholder="grep the filesystem from / …"
          preserveCase
          spellCheck={false}
          aria-label="grep pattern"
        />
        <Button variant="ghost" size="sm" onClick={grep} disabled={searching || !pattern.trim()}>
          {searching ? 'searching…' : 'grep'}
        </Button>
        <span className="cr-page-actions-gap" />
        {(['mkdir', 'rm', 'mv', 'chmod'] as OpKind[]).map((kind) => (
          <button
            key={kind}
            type="button"
            className="cr-page-linkish"
            onClick={() => {
              setOp(kind)
              setOpError(null)
            }}
          >
            {kind}
          </button>
        ))}
      </div>

      {treeError ? (
        <div className="cr-page-inline-error" role="alert">
          fs::ls failed: {treeError}
        </div>
      ) : null}

      <div className="cr-page-files-body">
        <div className="cr-page-tree" role="tree" aria-label="sandbox filesystem">
          {renderDir('/', 0)}
        </div>

        <div className="cr-page-viewer">
          {matches !== null ? (
            <div className="cr-page-grep-results">
              <div className="cr-page-grep-head">
                <span>
                  {matches.length} match{matches.length === 1 ? '' : 'es'} for{' '}
                  <code>{pattern}</code>
                </span>
                <button
                  type="button"
                  className="cr-page-linkish"
                  onClick={() => setMatches(null)}
                >
                  dismiss
                </button>
              </div>
              {matches.map((match, index) => (
                <button
                  // Matches have no id and can repeat per line — positional.
                  // biome-ignore lint/suspicious/noArrayIndexKey: positional rows
                  key={index}
                  type="button"
                  className="cr-page-grep-row"
                  onClick={() => match.path && openFile(match.path)}
                  disabled={!match.path}
                >
                  <span className="cr-page-grep-loc">
                    {match.path}
                    {match.line ? `:${match.line}` : ''}
                  </span>
                  <span className="cr-page-grep-text">{match.content}</span>
                </button>
              ))}
            </div>
          ) : null}

          {renderViewer()}
        </div>
      </div>

      <Dialog open={op !== null} onOpenChange={(open) => !open && setOp(null)}>
        {op === null || opField === null ? null : (
          <DialogContent className="cr-page-dialog">
            <DialogTitle>fs::{op}</DialogTitle>
            <div className="cr-page-dialog-fields">
              <Input
                value={opA}
                onChange={setOpA}
                placeholder={opField.a}
                preserveCase
                spellCheck={false}
                aria-label={opField.a}
              />
              {opField.b ? (
                <Input
                  value={opB}
                  onChange={setOpB}
                  placeholder={opField.b}
                  preserveCase
                  spellCheck={false}
                  aria-label={opField.b}
                />
              ) : null}
              {op === 'rm' ? (
                <label className="cr-page-checkline">
                  <input
                    type="checkbox"
                    checked={opRecursive}
                    onChange={(e) => setOpRecursive(e.target.checked)}
                  />
                  recursive
                </label>
              ) : null}
            </div>
            {opError ? (
              <div className="cr-page-inline-error" role="alert">
                {opError}
              </div>
            ) : null}
            <div className="cr-page-dialog-actions">
              <Button variant="ghost" size="sm" onClick={() => setOp(null)} disabled={opBusy}>
                cancel
              </Button>
              <Button
                variant="primary"
                size="sm"
                onClick={runOp}
                disabled={opBusy || !opA.trim() || (opField.b !== null && !opB.trim())}
              >
                {opBusy ? 'running…' : op}
              </Button>
            </div>
          </DialogContent>
        )}
      </Dialog>
    </div>
  )
}
