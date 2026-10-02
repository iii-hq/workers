/* "New worker…": a template, a name and a parent folder go to
   coder::scaffold-worker as `<folder>/<name>`; by default the worker is then
   added to the stack (compose::add) and followed until it runs, else the
   result offers "Add to stack". The steps and the stack calls live in
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
  StatusDot,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Check, ExternalLink, RefreshCw } from 'lucide-react'
import { type MouseEvent, useCallback, useEffect, useId, useReducer, useRef, useState } from 'react'
import { joinPath } from './coder'
import {
  addToStack,
  defaultDirectory,
  LANGUAGE_LABEL,
  type Language,
  type ListTemplatesResult,
  NEW_WORKER_INITIAL,
  newWorkerReducer,
  pickTemplate,
  type ScaffoldResult,
  sourceLabel,
  type Trigger,
  templateChoices,
  validateWorkerName,
} from './new-worker'

/** The http worker's default port; the -ade templates serve their public
    page at /<name> on it. */
const HTTP_PORT = 3111

/** Opens a tab in the browser worker: when it is registered, "Open public
    page" opens there, inside the console, instead of in a new browser tab. */
const BROWSER_START = 'browser::sessions::start'

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
  const [browser, setBrowser] = useState(false)
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

  // engine::functions::info answers NOT_FOUND when no browser worker runs.
  useEffect(() => {
    if (!host.panels) return
    host.iii.trigger('engine::functions::info', { function_id: BROWSER_START }).then(
      () => setBrowser(true),
      () => undefined,
    )
  }, [host])

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
        if (start) add(result, false)
      })
      .catch((error: unknown) => dispatch({ type: 'create-failed', error: errorMessage(error) }))
  }

  const result = state.result
  const publicHref = result ? `http://${window.location.hostname}:${HTTP_PORT}/${result.name}` : null

  // Opens the public page in the browser worker's console page; the browser
  // worker runs beside the http worker, so it opens 127.0.0.1. Without one, on
  // a modified click, or when no tab starts, the link opens a new tab as usual.
  function openInBrowser(event: MouseEvent<HTMLAnchorElement>) {
    if (!browser || !result || !publicHref || event.metaKey || event.ctrlKey || event.shiftKey) return
    event.preventDefault()
    host.iii
      .trigger<{ session_id: string }>(BROWSER_START, { url: `http://127.0.0.1:${HTTP_PORT}/${result.name}`, preview: false })
      .then(
        ({ session_id }) => {
          host.panels?.open({ pageId: 'browser', context: { sessionId: session_id } })
          onClose()
        },
        () => window.open(publicHref, '_blank', 'noreferrer'),
      )
  }

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
                <p className="shui-new-worker-status" role="alert">
                  <StatusDot tone="alert" />
                  Did not start: {state.error}
                </p>
                {state.logs.length > 0 ? <pre className="shui-new-worker-logs">{state.logs.join('\n')}</pre> : null}
              </>
            ) : (
              <p className="shui-new-worker-note">Not in the stack yet: add it to run it.</p>
            )}
            <div className="shui-text-dialog-actions">
              {state.step === 'running' && result.requires.includes('http') && publicHref ? (
                <Button asChild variant="ghost" size="sm">
                  <a href={publicHref} target="_blank" rel="noreferrer" onClick={openInBrowser}>
                    <ExternalLink aria-hidden />
                    Open public page
                  </a>
                </Button>
              ) : null}
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
