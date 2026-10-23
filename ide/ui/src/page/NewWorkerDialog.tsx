/* "New worker…": a template, a name and a parent folder go to
   coder::scaffold-worker as `<folder>/<name>`; by default the worker is then
   added to the stack (compose::add) and followed until it runs, else the
   result offers "Add to stack". The result shows the three steps, then the
   worker's functions and its pages. The steps and the stack calls live in
   new-worker.ts, which the tests drive. Mounted while open, so every opening
   starts from a fresh template list and empty fields. */

import {
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type Host,
  IconButton,
  Input,
  SegmentedControl,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Check, RefreshCw } from 'lucide-react'
import { type ReactNode, useCallback, useEffect, useId, useReducer, useRef, useState } from 'react'
import { joinPath } from './coder'
import {
  addToStack,
  composeOperations,
  defaultDirectory,
  entryFile,
  type FunctionEntry,
  formatElapsed,
  LANGUAGE_LABEL,
  type Language,
  type ListTemplatesResult,
  NEW_WORKER_INITIAL,
  newWorkerReducer,
  type ProgressStep,
  pickTemplate,
  type ScaffoldResult,
  sourceLabel,
  stackSteps,
  type Trigger,
  templateChoices,
  validateWorkerName,
  workerFunctions,
} from './new-worker'
import { StartFailure, StepList, WorkerFunctions } from './worker-result'

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
  const [start, setStart] = useState(true)
  const templateLabelId = useId()
  const templateName = useId()
  const nameId = useId()
  const nameHintId = useId()
  const folderId = useId()
  const folderHintId = useId()
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
  const picked = pickTemplate(template, templates)
  const languages = (['node', 'python'] as const).filter((lang) => templates.some((t) => t.language === lang))
  const language = templates.find((t) => t.id === picked)?.language ?? languages[0]
  const choices = language ? templateChoices(templates, language) : []
  const pickLanguage = (next: Language) => setTemplate(pickTemplate(undefined, templateChoices(templates, next)))
  const nameError = name === '' ? null : validateWorkerName(name)
  // An empty folder is the browsed root itself.
  const directory = defaultDirectory(folder.trim(), name || 'my-worker')
  const ready = state.step === 'form' && picked !== undefined && name !== '' && nameError === null

  const addedAt = useRef(0)
  // Closing stops following the add: its compose-operation binding goes.
  const following = useRef<AbortController | null>(null)
  useEffect(() => () => following.current?.abort(), [])
  const add = (result: ScaffoldResult, owned: boolean) => {
    addedAt.current = Date.now()
    dispatch({ type: 'add' })
    const trigger: Trigger = <T,>(functionId: string, payload: Record<string, unknown>) =>
      mounted.current ? host.iii.trigger<T>(functionId, payload) : Promise.reject<T>(new Error('the dialog closed'))
    following.current?.abort()
    const controller = new AbortController()
    following.current = controller
    void addToStack(trigger, composeOperations(host.iii), result, (phase) => dispatch({ type: 'progress', phase }), {
      owned,
      signal: controller.signal,
    }).then((outcome) =>
      dispatch(
        outcome.ok
          ? { type: 'added' }
          : { type: 'add-failed', error: outcome.error, logs: outcome.logs, owned: outcome.owned },
      ),
    )
  }

  const create = () => {
    if (!ready) return
    dispatch({ type: 'create' })
    host.iii
      .trigger<ScaffoldResult>('coder::scaffold-worker', {
        template: picked,
        name,
        directory: directory.startsWith('/') ? directory : joinPath(root, directory),
        // Files only: "Add to stack" below adds it, with its progress and Retry.
        start: false,
      })
      .then((result) => {
        dispatch({ type: 'created', result })
        onCreated(result)
        if (start) add(result, false)
      })
      .catch((error: unknown) => dispatch({ type: 'create-failed', error: errorMessage(error) }))
  }

  const result = state.result

  // The clock beside the step in progress: installs can take a minute.
  const [now, setNow] = useState(0)
  useEffect(() => {
    if (state.step !== 'adding') return
    setNow(Date.now())
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [state.step])

  // Once it runs, what it registered (functions::list leaves internal ones out).
  const [functions, setFunctions] = useState<FunctionEntry[] | null>(null)
  const runningName = state.step === 'running' ? result?.name : undefined
  useEffect(() => {
    if (!runningName) return
    host.iii.trigger<{ functions: FunctionEntry[] }>('engine::functions::list', {}).then(
      ({ functions: all }) => setFunctions(workerFunctions(all, runningName)),
      () => setFunctions([]),
    )
  }, [host, runningName])

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="shui-new-worker">
        <DialogTitle>{result ? result.name : 'New worker'}</DialogTitle>
        {result === null ? (
          <>
            <DialogDescription>Create an iii worker from a template.</DialogDescription>
            <form
              className="shui-new-worker-form"
              onSubmit={(event) => {
                event.preventDefault()
                create()
              }}
            >
              <div className="shui-new-worker-field">
                <div className="shui-new-worker-field-head">
                  <span id={templateLabelId} className="shui-text-dialog-label">
                    Template
                  </span>
                  {language && languages.length > 1 ? (
                    <SegmentedControl
                      aria-label="Language"
                      variant="radio"
                      value={language}
                      onChange={pickLanguage}
                      options={languages.map((lang) => ({ value: lang, label: LANGUAGE_LABEL[lang], icon: false }))}
                    />
                  ) : null}
                </div>
                <div className="shui-new-worker-templates" role="radiogroup" aria-labelledby={templateLabelId}>
                  {choices.map((choice) => (
                    <label key={choice.id} className="shui-new-worker-template">
                      <input
                        type="radio"
                        name={templateName}
                        value={choice.id}
                        checked={choice.id === picked}
                        onChange={() => setTemplate(choice.id)}
                        className="shui-sr-only"
                      />
                      <span className="shui-new-worker-template-text">
                        <span className="shui-new-worker-template-title">{choice.title}</span>
                        {choice.description ? (
                          <span className="shui-new-worker-template-description">{choice.description}</span>
                        ) : null}
                      </span>
                      <Check className="shui-new-worker-template-check" aria-hidden />
                    </label>
                  ))}
                  {choices.length === 0 ? (
                    <p className="shui-new-worker-templates-empty">
                      {state.step === 'loading' ? 'Loading templates…' : 'No templates found.'}
                    </p>
                  ) : null}
                </div>
                <div className="shui-new-worker-source">
                  <span title={state.list?.source.location}>
                    {state.list ? `From ${sourceLabel(state.list.source)}` : ''}
                  </span>
                  <IconButton
                    type="button"
                    label="Reload templates"
                    onClick={() => load(true)}
                    disabled={state.step !== 'form'}
                  >
                    <RefreshCw aria-hidden />
                  </IconButton>
                </div>
                {state.list?.source.warning ? (
                  <p className="shui-new-worker-note warn">{state.list.source.warning}</p>
                ) : null}
              </div>
              <div className="shui-new-worker-field">
                <label className="shui-text-dialog-label" htmlFor={nameId}>
                  Name
                </label>
                <Input
                  id={nameId}
                  value={name}
                  onChange={setName}
                  placeholder="my-worker"
                  spellCheck={false}
                  autoComplete="off"
                  aria-invalid={nameError !== null}
                  aria-describedby={nameHintId}
                  autoFocus
                />
                <p id={nameHintId} className={nameError ? 'shui-new-worker-note warn' : 'shui-new-worker-note'}>
                  {nameError ?? (
                    <>
                      Lowercase letters, digits and hyphens. Its functions are{' '}
                      <span className="shui-new-worker-token">{name || 'my-worker'}::…</span>
                    </>
                  )}
                </p>
              </div>
              <div className="shui-new-worker-field">
                <label className="shui-text-dialog-label" htmlFor={folderId}>
                  Parent folder
                </label>
                <Input
                  id={folderId}
                  value={folder}
                  onChange={setFolder}
                  placeholder="The open folder"
                  spellCheck={false}
                  autoComplete="off"
                  aria-describedby={folderHintId}
                />
                <p id={folderHintId} className="shui-new-worker-note">
                  Creates <span className="shui-new-worker-path">{directory}</span>
                </p>
              </div>
              <Checkbox
                className="shui-new-worker-start"
                label="Add it to the stack and start it"
                checked={start}
                onChange={(event) => setStart(event.target.checked)}
              />
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
                  {state.step === 'creating' ? 'Creating…' : start ? 'Create and start' : 'Create'}
                </Button>
              </div>
            </form>
          </>
        ) : (
          <>
            <DialogDescription>
              {templates.find((t) => t.id === result.template)?.name ?? result.template} in{' '}
              <span className="shui-new-worker-path">{result.directory}</span>
            </DialogDescription>
            <ResultSteps
              files={result.files.length}
              entry={entryLabel(result)}
              steps={stackSteps(state)}
              elapsed={state.step === 'adding' ? formatElapsed(now - addedAt.current) : null}
              failure={state.step === 'failed' ? <StartFailure error={state.error ?? ''} logs={state.logs} /> : null}
            />
            {state.step === 'result' ? (
              <p className="shui-new-worker-note">Not in the stack yet: add it to install and start it.</p>
            ) : null}
            {state.step === 'running' && functions && functions.length > 0 ? (
              <WorkerFunctions
                functions={functions}
                onTry={
                  host.chat?.openDraft
                    ? (id) => {
                        host.chat?.openDraft?.({ text: `Call ${id} and show me what it returns.`, title: `Try ${id}` })
                        onClose()
                      }
                    : undefined
                }
              />
            ) : null}
            <div className="shui-text-dialog-actions">
              {state.step === 'result' || state.step === 'failed' ? (
                <>
                  <Button type="button" variant="ghost" size="sm" onClick={onClose}>
                    Done
                  </Button>
                  <Button type="button" variant="primary" size="sm" onClick={() => add(result, state.owned)}>
                    {state.step === 'failed' ? 'Retry' : 'Add to stack'}
                  </Button>
                </>
              ) : (
                <Button
                  type="button"
                  variant={state.step === 'running' ? 'primary' : 'ghost'}
                  size="sm"
                  onClick={onClose}
                >
                  Done
                </Button>
              )}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

/** The entry file onCreated opened, relative to the worker's folder. */
function entryLabel(result: ScaffoldResult): string | null {
  const entry = entryFile(result.files.map((file) => file.path))
  return entry?.startsWith(`${result.directory}/`) ? entry.slice(result.directory.length + 1) : entry
}

/** Create, Install, Start: done, in progress (with its clock), waiting or failed. */
function ResultSteps({
  files,
  entry,
  steps: [install, start],
  elapsed,
  failure,
}: {
  files: number
  entry: string | null
  steps: [ProgressStep, ProgressStep]
  elapsed: string | null
  failure: ReactNode
}) {
  return (
    <StepList
      rows={[
        {
          key: 'files',
          label: `Created ${files} ${files === 1 ? 'file' : 'files'}`,
          state: 'done',
          detail: entry && `${entry} is open`,
        },
        { key: 'install', ...install, detail: install.state === 'active' ? elapsed : null, failure },
        { key: 'start', ...start, detail: start.state === 'active' ? elapsed : null, failure },
      ]}
    />
  )
}
