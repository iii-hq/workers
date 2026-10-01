/* "New worker…": a template, a name and a parent folder go to
   coder::scaffold-worker as `<folder>/<name>`; the result then offers "Add
   to stack" (compose::add) and follows the worker until it runs. The steps
   and the stack calls live in new-worker.ts, which the tests drive. Mounted
   while open, so every opening starts from a fresh template list and empty
   fields. */

import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  IconButton,
  Input,
  Select,
  StatusDot,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ExternalLink, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useId, useReducer, useRef, useState } from 'react'
import { joinPath } from './coder'
import {
  addToStack,
  defaultDirectory,
  type ListTemplatesResult,
  NEW_WORKER_INITIAL,
  newWorkerReducer,
  type ScaffoldResult,
  sourceLabel,
  type Trigger,
  validateWorkerName,
} from './new-worker'

/** The http worker's default bind; the -ade templates serve their page at /<name>. */
const HTTP_BASE = 'http://127.0.0.1:3111'

export interface NewWorkerDialogProps {
  host: Host
  /** The browsed root, canonical absolute: a relative folder resolves against it. */
  root: string
  /** The Folder field's first value: the root-relative parent folder. */
  baseDir: string
  /** The files are on disk. */
  onCreated: (result: ScaffoldResult) => void
  onClose: () => void
}

export function NewWorkerDialog({ host, root, baseDir, onCreated, onClose }: NewWorkerDialogProps) {
  const [state, dispatch] = useReducer(newWorkerReducer, NEW_WORKER_INITIAL)
  const [template, setTemplate] = useState<string>()
  const [name, setName] = useState('')
  // The parent folder; the worker's own folder is always `<folder>/<name>`
  // (coder::scaffold-worker refuses any other last segment, C232).
  const [folder, setFolder] = useState(baseDir)
  const templateId = useId()
  const nameId = useId()
  const folderId = useId()
  // Closing stops a running "Add to stack" at its next bus call. Set in the
  // effect body too: Strict Mode replays the cleanup on mount.
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  const load = useCallback(
    (refresh: boolean) => {
      dispatch({ type: 'load' })
      host.iii
        .trigger<ListTemplatesResult>('coder::list-templates', { refresh })
        .then((list) => dispatch({ type: 'listed', list }))
        .catch((error: unknown) => dispatch({ type: 'list-failed', error: errorMessage(error) }))
    },
    [host],
  )
  useEffect(() => load(false), [load])

  const templates = state.list?.templates ?? []
  const picked = template ?? (templates.find((t) => t.id.endsWith('-ade')) ?? templates[0])?.id
  const groups = (['node', 'python'] as const)
    .map((language) => ({
      label: language === 'node' ? 'Node' : 'Python',
      options: templates
        .filter((t) => t.language === language)
        .map((t) => ({ value: t.id, label: t.name, description: t.description })),
    }))
    .filter((group) => group.options.length > 0)
  const nameError = name === '' ? null : validateWorkerName(name)
  // An empty folder is the browsed root itself.
  const directory = defaultDirectory(folder.trim(), name || 'my-worker')
  const ready = state.step === 'form' && picked !== undefined && name !== '' && nameError === null

  const create = () => {
    if (!ready) return
    dispatch({ type: 'create' })
    host.iii
      .trigger<ScaffoldResult>('coder::scaffold-worker', {
        template: picked,
        name,
        directory: directory.startsWith('/') ? directory : joinPath(root, directory),
      })
      .then((result) => {
        dispatch({ type: 'created', result })
        onCreated(result)
      })
      .catch((error: unknown) => dispatch({ type: 'create-failed', error: errorMessage(error) }))
  }

  const add = (result: ScaffoldResult, owned: boolean) => {
    dispatch({ type: 'add' })
    const trigger: Trigger = <T,>(functionId: string, payload: Record<string, unknown>) =>
      mounted.current ? host.iii.trigger<T>(functionId, payload) : Promise.reject<T>(new Error('the dialog closed'))
    void addToStack(trigger, result, (phase) => dispatch({ type: 'progress', phase }), { owned }).then((outcome) =>
      dispatch(
        outcome.ok
          ? { type: 'added' }
          : { type: 'add-failed', error: outcome.error, logs: outcome.logs, owned: outcome.owned },
      ),
    )
  }

  const result = state.result
  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="shui-new-worker">
        <DialogTitle>New worker</DialogTitle>
        {result === null ? (
          <>
            <DialogDescription>Create an iii worker from a template.</DialogDescription>
            <form
              className="shui-text-dialog-form"
              onSubmit={(event) => {
                event.preventDefault()
                create()
              }}
            >
              <div className="shui-new-worker-source">
                <span title={state.list?.source.location}>
                  {state.list ? sourceLabel(state.list.source) : 'Loading templates…'}
                </span>
                <IconButton
                  type="button"
                  label="Refresh templates"
                  onClick={() => load(true)}
                  disabled={state.step !== 'form'}
                >
                  <RefreshCw aria-hidden />
                </IconButton>
              </div>
              {state.list?.source.warning ? (
                <p className="shui-new-worker-note warn">{state.list.source.warning}</p>
              ) : null}
              <label className="shui-text-dialog-label" htmlFor={templateId}>
                Template
              </label>
              <Select
                id={templateId}
                value={picked}
                groups={groups}
                onChange={setTemplate}
                placeholder={state.step === 'loading' ? 'Loading…' : 'No templates'}
                disabled={templates.length === 0}
              />
              <label className="shui-text-dialog-label" htmlFor={nameId}>
                Name
              </label>
              <Input
                id={nameId}
                value={name}
                onChange={setName}
                placeholder="my-worker"
                spellCheck={false}
                aria-invalid={nameError !== null}
                autoFocus
              />
              <p className={nameError ? 'shui-new-worker-note warn' : 'shui-new-worker-note'}>
                {nameError ?? `Functions: ${name || 'my-worker'}::…`}
              </p>
              <label className="shui-text-dialog-label" htmlFor={folderId}>
                Folder
              </label>
              <Input id={folderId} value={folder} onChange={setFolder} spellCheck={false} />
              <p className="shui-new-worker-note">Creates {directory}</p>
              {state.error ? (
                <p className="shui-new-worker-note warn" role="alert">
                  {state.error}
                </p>
              ) : null}
              <div className="shui-text-dialog-actions">
                <Button type="button" variant="ghost" size="sm" onClick={onClose}>
                  Cancel
                </Button>
                <Button type="submit" variant="primary" size="sm" disabled={!ready}>
                  {state.step === 'creating' ? 'Creating…' : 'Create'}
                </Button>
              </div>
            </form>
          </>
        ) : (
          <>
            <DialogDescription>
              {result.files.length} {result.files.length === 1 ? 'file' : 'files'} created in{' '}
              <span className="shui-new-worker-path">{result.directory}</span>
            </DialogDescription>
            {state.step === 'adding' ? (
              <p className="shui-new-worker-status" role="status">
                <StatusDot tone="accent" pulse />
                {state.phase === 'installing' ? 'Installing…' : 'Starting…'}
              </p>
            ) : state.step === 'running' ? (
              <p className="shui-new-worker-status" role="status">
                <StatusDot tone="ok" />
                Running
              </p>
            ) : state.step === 'failed' ? (
              <>
                <p className="shui-new-worker-note warn" role="alert">
                  {state.error}
                </p>
                {state.logs.length > 0 ? <pre className="shui-new-worker-logs">{state.logs.join('\n')}</pre> : null}
              </>
            ) : null}
            <div className="shui-text-dialog-actions">
              {state.step === 'running' && result.requires.includes('http') ? (
                <Button asChild variant="ghost" size="sm">
                  <a href={`${HTTP_BASE}/${result.name}`} target="_blank" rel="noreferrer">
                    <ExternalLink aria-hidden />
                    Open in browser
                  </a>
                </Button>
              ) : null}
              <Button type="button" variant="ghost" size="sm" onClick={onClose}>
                Done
              </Button>
              {state.step === 'result' || state.step === 'failed' ? (
                <Button type="button" variant="primary" size="sm" onClick={() => add(result, state.owned)}>
                  {state.step === 'failed' ? 'Retry' : 'Add to stack'}
                </Button>
              ) : null}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
