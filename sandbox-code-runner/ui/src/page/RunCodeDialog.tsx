/**
 * The "run code" dialog: `sandbox-code-runner::run` with a real editor —
 * language select (node / python / shell), keep-the-sandbox toggle, and an
 * optional existing-runtime target (running into a kept runtime reuses its
 * warm interpreter and installed state). The process result is reported
 * with the sandbox it actually ran in (known only when targeting an
 * existing runtime), so the caller can route it to the right timeline.
 */

import {
  Button,
  CodeEditor,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  Select,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { parseExecResult } from './exec'
import type { ExecRecord } from './records'
import type { RuntimeInfo } from './store'

type Lang = 'node' | 'python' | 'shell'

const EDITOR_LANG: Record<Lang, string> = {
  node: 'javascript',
  python: 'python',
  shell: 'shell',
}

let runSeq = 0

/** Targeting a runtime pins the editor to that runtime's own language;
 *  a runtime whose lang this dialog cannot offer gets plaintext rather
 *  than a wrong highlight. */
function editorLanguage(
  targetLang: string | undefined,
  runtimeId: string | undefined,
  lang: Lang,
): string {
  if (targetLang && targetLang in EDITOR_LANG) {
    return EDITOR_LANG[targetLang as Lang]
  }
  if (runtimeId) return 'plaintext'
  return EDITOR_LANG[lang]
}

export function RunCodeDialog({
  host,
  open,
  runtimes,
  onClose,
  onResult,
}: {
  host: Host
  open: boolean
  runtimes: RuntimeInfo[]
  onClose(): void
  /** The run's process result, plus the sandbox it ran in — `null` for a
   *  fresh sandbox (one-shot or kept), whose id the response does not
   *  carry. The caller decides which timeline (if any) it lands in. */
  onResult(record: ExecRecord, sandboxId: string | null): void
}) {
  const [code, setCode] = useState('')
  const [lang, setLang] = useState<Lang>('node')
  const [keep, setKeep] = useState(false)
  const [runtimeId, setRuntimeId] = useState<string | undefined>(undefined)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // A previous open's failure must not greet the next one.
  useEffect(() => {
    if (open) setError(null)
  }, [open])

  const runtimeOptions = runtimes.map((rt) => ({
    value: rt.runtime_id,
    label: `${rt.lang} · ${rt.runtime_id.slice(0, 11)}…`,
  }))

  // The targeted runtime can leave the live list (reaped, torn down)
  // while the dialog sits open — drop the selection during render so the
  // Select and the run() payload always agree.
  if (runtimeId && !runtimes.some((rt) => rt.runtime_id === runtimeId)) {
    setRuntimeId(undefined)
  }

  const targetLang = runtimeId
    ? runtimes.find((rt) => rt.runtime_id === runtimeId)?.lang
    : undefined
  const editorLang = editorLanguage(targetLang, runtimeId, lang)

  const run = () => {
    if (!code.trim() || busy) return
    const payload: Record<string, unknown> = { code }
    if (runtimeId) payload.runtime_id = runtimeId
    else payload.lang = lang
    if (keep) payload.keep = true

    setBusy(true)
    setError(null)
    host.iii
      .trigger('sandbox-code-runner::run', payload, { timeoutMs: 120_000 })
      .then((value) => {
        const result = parseExecResult(value)
        const firstLine = code.trim().split('\n')[0].slice(0, 80)
        const ranIn = runtimeId
          ? (runtimes.find((rt) => rt.runtime_id === runtimeId)?.sandbox_id ?? null)
          : null
        onResult(
          {
            id: `run-${Date.now()}-${runSeq++}`,
            at: Date.now(),
            cmd: `run ${runtimeId ? 'runtime' : lang}: ${firstLine}`,
            shell: false,
            detached: false,
            source: 'run',
            ...result,
          },
          ranIn,
        )
        onClose()
      })
      .catch((err: unknown) =>
        setError(err instanceof Error ? err.message : String(err)),
      )
      .finally(() => setBusy(false))
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && !busy && onClose()}>
      {!open ? null : (
        <DialogContent className="cr-page-dialog cr-page-run-dialog">
          <DialogTitle>run code</DialogTitle>
          <DialogDescription>
            sandbox-code-runner::run — a fresh throwaway sandbox unless you
            keep it or target an existing runtime.
          </DialogDescription>
          <div className="cr-page-field-row">
            <Select
              value={runtimeId ? undefined : lang}
              options={[
                { value: 'node', label: 'node' },
                { value: 'python', label: 'python' },
                { value: 'shell', label: 'shell' },
              ]}
              onChange={(next) => setLang(next as Lang)}
              disabled={runtimeId !== undefined}
              aria-label="language"
              placeholder="lang"
            />
            {runtimeOptions.length > 0 ? (
              <Select
                value={runtimeId}
                options={runtimeOptions}
                onChange={setRuntimeId}
                placeholder="fresh sandbox"
                allowEmpty
                emptyLabel="fresh sandbox"
                onClear={() => setRuntimeId(undefined)}
                aria-label="target runtime"
              />
            ) : null}
            <button
              type="button"
              className={`cr-page-toggle${keep ? ' on' : ''}`}
              onClick={() => setKeep((v) => !v)}
              aria-pressed={keep}
              title="keep the sandbox (and its runtime) alive after the run, so state and registered functions persist"
            >
              keep
            </button>
          </div>
          <div className="cr-page-editor-pane">
            <CodeEditor
              value={code}
              onChange={setCode}
              language={editorLang}
              className="cr-page-payload-editor"
              placeholder="code to run"
              aria-label="code"
              autoFocus
            />
          </div>
          {error ? (
            <div className="cr-page-inline-error" role="alert">
              {error}
            </div>
          ) : null}
          <div className="cr-page-dialog-actions">
            <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
              cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={run}
              disabled={busy || !code.trim()}
            >
              {busy ? 'running…' : 'run'}
            </Button>
          </div>
        </DialogContent>
      )}
    </Dialog>
  )
}
