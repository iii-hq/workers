import { Button, type Host, Input, Select } from '@iii-dev/console-ui'
import { useEffect, useMemo, useState } from 'react'
import { errText } from './errors.js'
import { FOLLOW_CHAT, modelPickerOptions, requestedModel, selectionIsStale } from './model-picker.js'
import {
  type CatalogModel,
  loadComposerModel,
  loadModelCatalog,
  loadScanFormDefaults,
  normalizeCommitSha,
  requestNewRun,
  type ScanMode,
  subscribeModelCatalog,
  supportsLiveComposerModel,
} from './security-scan-data'

const SCAN_MODE_OPTIONS: Array<{ value: ScanMode; label: string }> = [
  { value: 'scan', label: 'scan (report only)' },
  { value: 'suggest', label: 'suggest (include patches)' },
]
const SESSION_META_FN = 'security-scan-ui::session-meta'
const SESSION_CREATED_FN = 'security-scan-ui::session-created'

function analysisModelLabel(composerModel: string | null, operatorModel: string | null): string {
  if (composerModel) return composerModel
  if (operatorModel) return `operator default: ${operatorModel}`
  return 'operator default'
}

function analysisModelHint(selection: string, composerModel: string | null, composerApi: boolean): string {
  if (selection !== FOLLOW_CHAT) {
    return 'This scan runs on the model chosen here, whatever the chat composer holds.'
  }
  if (composerModel) {
    return 'Following the open chat composer. Pick a model above to pin this scan instead.'
  }
  if (composerApi) {
    return 'The open chat has no composer model yet, so this scan uses the operator default. Pick a model above to pin it.'
  }
  return 'This Console build does not expose the chat composer model, so this scan uses the operator default. Pick a model above to pin it.'
}

export function ScanRequestForm({
  host,
  conversationId,
  onStarted,
}: {
  host: Host
  conversationId?: string | null
  onStarted: (runId: string) => void
}) {
  const [repositories, setRepositories] = useState<string[]>([])
  const [repository, setRepository] = useState('')
  const [targetSha, setTargetSha] = useState('')
  const [mode, setMode] = useState<ScanMode>('scan')
  const [operatorModel, setOperatorModel] = useState<string | null>(null)
  const [composerModel, setComposerModel] = useState<string | null>(null)
  const [catalog, setCatalog] = useState<CatalogModel[]>([])
  const [selectedModel, setSelectedModel] = useState<string>(FOLLOW_CHAT)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const analysisModel = composerModel || operatorModel
  const composerApi = supportsLiveComposerModel(host)
  const modelOptions = useMemo(
    () => modelPickerOptions(catalog, analysisModelLabel(composerModel, operatorModel)),
    [catalog, composerModel, operatorModel],
  )

  useEffect(() => {
    let cancelled = false
    void loadScanFormDefaults(host).then((defaults) => {
      if (cancelled) return
      setRepositories(defaults.repositories)
      setOperatorModel(defaults.analysisModel)
      setRepository((current) => current || defaults.repositories[0] || '')
    })
    return () => {
      cancelled = true
    }
  }, [host])

  useEffect(() => {
    let cancelled = false
    void loadComposerModel(host, conversationId).then((model) => {
      if (!cancelled) setComposerModel(model)
    })
    return () => {
      cancelled = true
    }
  }, [conversationId, host])

  useEffect(() => {
    let cancelled = false
    const refresh = () => {
      void loadModelCatalog(host).then((models) => {
        if (!cancelled) setCatalog(models)
      })
    }
    refresh()
    const dispose = subscribeModelCatalog(host, refresh)
    return () => {
      cancelled = true
      dispose()
    }
  }, [host])

  useEffect(() => {
    if (selectionIsStale(catalog, selectedModel)) setSelectedModel(FOLLOW_CHAT)
  }, [catalog, selectedModel])

  useEffect(() => {
    const sessionId = conversationId?.trim()
    if (!sessionId) return
    const metaFn = `${SESSION_META_FN}::${sessionId}`
    const createdFn = `${SESSION_CREATED_FN}::${sessionId}`
    const offMeta = host.iii.on<{
      session_id?: string
      metadata?: Record<string, unknown>
    }>(metaFn, (event) => {
      if (!event || event.session_id !== sessionId) return
      const model = event.metadata?.model
      setComposerModel(typeof model === 'string' && model.trim() ? model.trim() : null)
    })
    const offCreated = host.iii.on<{ session_id?: string }>(createdFn, (event) => {
      if (!event || event.session_id !== sessionId) return
      void loadComposerModel(host, sessionId).then(setComposerModel)
    })
    const offMetaTrigger = host.iii.registerTrigger({
      type: 'session::meta-updated',
      function_id: `${metaFn}::${host.iii.browserId}`,
      config: {},
    })
    const offCreatedTrigger = host.iii.registerTrigger({
      type: 'session::created',
      function_id: `${createdFn}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offMetaTrigger()
      offCreatedTrigger()
      offMeta()
      offCreated()
    }
  }, [host, conversationId])

  const submit = async () => {
    const sha = normalizeCommitSha(targetSha)
    if (!repository.trim()) {
      setError('Choose an allowlisted repository.')
      return
    }
    if (targetSha.trim() && !sha) {
      setError('Commit SHA must be 40 hexadecimal characters, or leave it blank for the entire repository.')
      return
    }
    setPending(true)
    setError(null)
    try {
      let liveComposerModel = composerModel
      if (selectedModel === FOLLOW_CHAT) {
        liveComposerModel = await loadComposerModel(host, conversationId)
        setComposerModel(liveComposerModel)
      }
      const model = requestedModel(selectedModel, liveComposerModel)
      const result = await requestNewRun(host, {
        repository: repository.trim(),
        ...(sha ? { target_sha: sha } : {}),
        mode,
        ...(model ? { model } : {}),
      })
      setTargetSha('')
      onStarted(result.run_id)
    } catch (caught) {
      setError(errText(caught))
    } finally {
      setPending(false)
    }
  }

  return (
    <form
      className="security-scan-ui-new-run"
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      <div className="security-scan-ui-filter-head">
        <span>new scan</span>
      </div>
      <div className="security-scan-ui-filter">
        <label htmlFor="security-scan-new-repository">repository</label>
        {repositories.length > 0 ? (
          <Select
            value={repository || repositories[0]}
            options={repositories.map((id) => ({ value: id, label: id }))}
            onChange={setRepository}
            aria-label="Allowlisted repository"
          />
        ) : (
          <Input
            id="security-scan-new-repository"
            value={repository}
            onChange={setRepository}
            placeholder="allowlisted repository id"
            preserveCase
            spellCheck={false}
          />
        )}
      </div>
      <div className="security-scan-ui-filter">
        <label htmlFor="security-scan-new-sha">commit SHA</label>
        <Input
          id="security-scan-new-sha"
          value={targetSha}
          onChange={setTargetSha}
          placeholder="blank = entire repo analysis"
          preserveCase
          spellCheck={false}
        />
        <p className="security-scan-ui-new-run-hint">
          {targetSha.trim()
            ? 'Reviews the full repository tree at this commit, not the commit diff and not git history.'
            : 'No SHA entered: this will do entire repo analysis at HEAD.'}
        </p>
      </div>
      <div className="security-scan-ui-filter">
        <span id="security-scan-new-mode-label">mode</span>
        <Select
          value={mode}
          options={SCAN_MODE_OPTIONS}
          onChange={(value) => setMode(value === 'suggest' ? 'suggest' : 'scan')}
          aria-label="Scan mode"
        />
      </div>
      <div className="security-scan-ui-filter">
        <span id="security-scan-new-model-label">analysis model</span>
        {catalog.length > 0 ? (
          <Select
            value={selectedModel}
            options={modelOptions}
            onChange={setSelectedModel}
            aria-label="Analysis model"
          />
        ) : (
          <p className="security-scan-ui-new-run-model" title={analysisModel ?? undefined}>
            {analysisModelLabel(composerModel, operatorModel)}
          </p>
        )}
        <p className="security-scan-ui-new-run-hint">
          {catalog.length > 0
            ? analysisModelHint(selectedModel, composerModel, composerApi)
            : 'The router catalog is unavailable, so this scan uses the model the chat composer holds, or the operator default.'}
        </p>
      </div>
      {error ? (
        <p className="security-scan-ui-new-run-error" role="alert">
          {error}
        </p>
      ) : null}
      <Button type="submit" disabled={pending}>
        {pending ? 'starting…' : 'start scan'}
      </Button>
    </form>
  )
}
