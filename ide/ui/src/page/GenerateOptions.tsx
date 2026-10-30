/* Generate's options, beside the button: a shadcn popover naming the model
   and reasoning effort that write the message (each a row that opens its
   own list, and a pick saves at once into the worker's `commit_messages`
   settings) and the repository instructions, with the full settings a
   click away. */

import type { Host } from '@iii-dev/console-ui'
import {
  IconButton,
  List,
  Popover,
  PopoverContent,
  PopoverTrigger,
  SearchField,
  Separator,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, Check, ChevronDown, ChevronRight, SlidersHorizontal } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { type CommitMessageConfig, modelLabel } from './use-commit-message'

/** The configuration entry the Generate settings live in. */
export const CONFIGURATION_ID = 'ide'

interface ModelChoice {
  id: string
  label: string
  provider: string
}

const THINKING: readonly { value: string; label: string; detail: string }[] = [
  { value: 'default', label: 'Model default', detail: 'No reasoning override' },
  { value: 'minimal', label: 'Minimal', detail: 'Fastest, barely thinks' },
  { value: 'low', label: 'Low', detail: 'Quick, enough for most diffs' },
  { value: 'medium', label: 'Medium', detail: 'Balanced' },
  { value: 'high', label: 'High', detail: 'Slower, for large or subtle changes' },
  { value: 'xhigh', label: 'Extra high', detail: 'Slowest, most thorough' },
]

const thinkingLabel = (value: string | undefined) =>
  THINKING.find((option) => option.value === value)?.label ?? value ?? 'Model default'

/** `router::models::list` rows as choices: `provider::id` unless already qualified. */
export function modelChoices(response: unknown): ModelChoice[] {
  const models = (response as { models?: unknown } | null | undefined)?.models
  if (!Array.isArray(models)) return []
  return models.flatMap((raw) => {
    if (!raw || typeof raw !== 'object') return []
    const row = raw as Record<string, unknown>
    const id = typeof row.id === 'string' ? row.id.trim() : ''
    if (!id) return []
    const provider = typeof row.provider === 'string' ? row.provider.trim() : ''
    const label = typeof row.display_name === 'string' && row.display_name.trim() ? row.display_name.trim() : id
    const full = provider && !id.includes('::') ? `${provider}::${id}` : id
    return [{ id: full, label, provider: provider || full.split('::')[0] }]
  })
}

const asObject = (value: unknown): Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}

/** The IDE's own configuration entry. Compose names it per project
    (`<namespace>-ide`, from `III_CONFIG_NAME`); the worker marks it with the
    `ide` form, so that is what finds it. */
async function ideConfigurationId(host: Host): Promise<string> {
  const listed = await host.iii.call<{ configurations?: unknown }>('configuration::list', {})
  const entries = Array.isArray(listed?.configurations) ? listed.configurations.map(asObject) : []
  const own =
    entries.find((entry) => entry.id === CONFIGURATION_ID) ??
    entries.find((entry) => asObject(entry.metadata).ui_form === CONFIGURATION_ID)
  if (typeof own?.id !== 'string') throw new Error('The IDE’s settings entry was not found')
  return own.id
}

/** Read the stored settings, set `commit_messages.<key>`, write them back;
    the worker reads them live, so the next Generate uses the change. */
async function saveCommitSetting(host: Host, patch: Partial<Pick<CommitMessageConfig, 'model' | 'thinking'>>) {
  const id = await ideConfigurationId(host)
  const response = await host.iii.call<{ value?: unknown }>('configuration::get', { id, raw: true })
  const current = asObject(response?.value)
  const next = { ...current, commit_messages: { ...asObject(current.commit_messages), ...patch } }
  await host.iii.call('configuration::set', { id, value: next })
}

type Page = 'main' | 'model' | 'thinking'

export function GenerateOptions({
  host,
  config: read,
  model: readModel,
  usesChatDefault: readUsesChat,
  chatModel,
  onReload,
  onOpenSettings,
}: {
  host: Host
  /** The worker's settings; null on a worker without them (only Settings then). */
  config: CommitMessageConfig | null
  /** The model Generate will use. */
  model: string | null
  usesChatDefault: boolean
  /** The chat composer's model, used when none is set here. */
  chatModel: string | null
  onReload: () => void
  onOpenSettings: () => void
}) {
  const [open, setOpen] = useState(false)
  const [page, setPage] = useState<Page>('main')
  const [models, setModels] = useState<ModelChoice[] | null>(null)
  const [query, setQuery] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const pageRef = useRef<HTMLDivElement>(null)
  // What was just saved shows at once, until a read of the worker's settings
  // has it too: the worker applies a write a moment after it lands.
  const [saved, setSaved] = useState<Partial<Pick<CommitMessageConfig, 'model' | 'thinking'>> | null>(null)
  useEffect(() => {
    if (read === null || saved === null) return
    const caughtUp = (Object.keys(saved) as (keyof typeof saved)[]).every((key) => read[key] === saved[key])
    if (caughtUp) setSaved(null)
  }, [read, saved])
  const config = read && saved ? { ...read, ...saved } : read
  const pickedModel = saved !== null && 'model' in saved
  const model = pickedModel ? (saved.model ?? chatModel) : readModel
  const usesChatDefault = pickedModel ? saved.model === null : readUsesChat

  // The catalog, read once a popover opens on it.
  useEffect(() => {
    if (!open || models !== null) return
    let live = true
    Promise.resolve()
      .then(() => host.iii.call('router::models::list', {}, { timeoutMs: 10_000 }))
      .then(modelChoices, () => [])
      .then((choices) => live && setModels(choices))
    return () => {
      live = false
    }
  }, [open, models, host])

  // A page that opens takes the focus: the model search, or the thinking pick.
  useEffect(() => {
    if (!open || page === 'main') return
    requestAnimationFrame(() =>
      pageRef.current
        ?.querySelector<HTMLElement>('input, [aria-pressed="true"], [data-list-item]')
        ?.focus({ preventScroll: true }),
    )
  }, [open, page])

  const save = async (patch: Partial<Pick<CommitMessageConfig, 'model' | 'thinking'>>) => {
    setSaving(true)
    setError(null)
    try {
      await saveCommitSetting(host, patch)
      setSaved((previous) => ({ ...previous, ...patch }))
      onReload()
      setTimeout(onReload, 1000)
      setPage('main')
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setSaving(false)
    }
  }

  const stored = config?.model ?? null
  // The catalog's name for it (Claude Sonnet 5.5) once read, else the id.
  const modelName = model ? (models?.find((choice) => choice.id === model)?.label ?? modelLabel(model)) : 'None'
  const lowered = query.trim().toLowerCase()
  const shownModels = (models ?? []).filter(
    (choice) => lowered === '' || `${choice.label} ${choice.id}`.toLowerCase().includes(lowered),
  )
  // The stored model stays pickable when the catalog does not list it.
  if (stored !== null && models !== null && !models.some((choice) => choice.id === stored) && lowered === '') {
    shownModels.unshift({ id: stored, label: modelLabel(stored), provider: stored.split('::')[0] })
  }

  const back = (
    <IconButton label="Back" variant="ghost" onClick={() => setPage('main')}>
      <ArrowLeft aria-hidden />
    </IconButton>
  )
  const choiceRow = (key: string, checked: boolean, label: string, meta: string | null, onPick: () => void) => (
    <button
      key={key}
      type="button"
      aria-pressed={checked}
      data-list-item=""
      className="shui-gen-choice"
      disabled={saving}
      onClick={() => (checked ? setPage('main') : onPick())}
    >
      <span className="shui-gen-choice-text">
        <span className="shui-gen-choice-label">{label}</span>
        {meta ? <span className="shui-gen-choice-meta">{meta}</span> : null}
      </span>
      {checked ? <Check aria-hidden className="shui-gen-choice-check" /> : null}
    </button>
  )

  const mainPage = (
    <>
      <div className="shui-gen-head">
        <span className="shui-gen-title">Commit message</span>
        <span className="shui-gen-sub">Written from the ticked changes</span>
      </div>
      <div className="shui-gen-rows">
        <button
          type="button"
          className="shui-gen-row"
          disabled={config === null}
          aria-label={`Model: ${modelName}${usesChatDefault && model ? ', chat default' : ''}. Change`}
          onClick={() => setPage('model')}
        >
          <span className="shui-gen-row-label">Model</span>
          <span className="shui-gen-row-value" data-stacked={(usesChatDefault && model) || undefined}>
            <span className="shui-gen-row-model">{modelName}</span>
            {usesChatDefault && model ? <span className="shui-gen-row-hint">Chat default</span> : null}
          </span>
          <ChevronRight aria-hidden className="shui-gen-row-caret" />
        </button>
        <button
          type="button"
          className="shui-gen-row"
          disabled={config === null}
          aria-label={`Thinking: ${thinkingLabel(config?.thinking)}. Change`}
          onClick={() => setPage('thinking')}
        >
          <span className="shui-gen-row-label">Thinking</span>
          <span className="shui-gen-row-value">{thinkingLabel(config?.thinking)}</span>
          <ChevronRight aria-hidden className="shui-gen-row-caret" />
        </button>
      </div>
      <Separator className="shui-gen-sep" />
      <div className="shui-gen-section">
        <div className="shui-gen-section-head">
          <span className="shui-gen-section-label">Instructions</span>
          <button
            type="button"
            className="shui-gen-link"
            onClick={() => {
              setOpen(false)
              onOpenSettings()
            }}
          >
            {config?.instructions.trim() ? 'Edit' : 'Add'}
          </button>
        </div>
        <p className="shui-gen-instructions" data-empty={!config?.instructions.trim() || undefined}>
          {config?.instructions.trim() || 'None yet. Add your conventions, e.g. “Use Conventional Commits”.'}
        </p>
      </div>
      <Separator className="shui-gen-sep" />
      <div className="shui-gen-foot">
        <button
          type="button"
          className="shui-gen-foot-button"
          onClick={() => {
            setOpen(false)
            onOpenSettings()
          }}
        >
          <SlidersHorizontal aria-hidden />
          Commit message settings…
        </button>
      </div>
    </>
  )

  const modelPage = (
    <>
      <div className="shui-gen-page-head">
        {back}
        <span className="shui-gen-title">Model</span>
      </div>
      <div className="shui-gen-search">
        <SearchField
          className="shui-gen-filter"
          value={query}
          onChange={setQuery}
          placeholder="Search models"
          aria-label="Search models"
          autoComplete="off"
          spellCheck={false}
          onKeyDown={(event) => {
            if (event.key !== 'ArrowDown') return
            event.preventDefault()
            pageRef.current?.querySelector<HTMLElement>('[data-list-item]:not([disabled])')?.focus()
          }}
        />
      </div>
      <List className="shui-gen-list" aria-label="Models">
        {lowered === ''
          ? choiceRow(
              'chat-default',
              stored === null,
              'Chat default',
              chatModel ? `Follows the chat · ${modelLabel(chatModel)}` : 'Follows the chat’s model',
              () => void save({ model: null }),
            )
          : null}
        {lowered === '' && shownModels.length > 0 ? <Separator className="shui-gen-sep" /> : null}
        {models === null ? (
          <p className="shui-gen-note">Reading models…</p>
        ) : shownModels.length === 0 ? (
          <p className="shui-gen-note">{lowered ? 'No model matches.' : 'No models. Add a provider in Settings.'}</p>
        ) : (
          shownModels.map((choice) =>
            choiceRow(choice.id, choice.id === stored, choice.label, choice.provider, () => void save({ model: choice.id })),
          )
        )}
      </List>
    </>
  )

  const thinkingPage = (
    <>
      <div className="shui-gen-page-head">
        {back}
        <span className="shui-gen-title">Thinking</span>
      </div>
      <List className="shui-gen-list" aria-label="Thinking">
        {THINKING.map((option) =>
          choiceRow(option.value, option.value === (config?.thinking ?? 'low'), option.label, option.detail, () =>
            void save({ thinking: option.value }),
          ),
        )}
      </List>
    </>
  )

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) return
        onReload()
        setPage('main')
        setQuery('')
        setError(null)
      }}
    >
      <PopoverTrigger
        render={
          <IconButton label="Generate options" className="shui-generate-more">
            <ChevronDown aria-hidden />
          </IconButton>
        }
      />
      <PopoverContent align="end" sideOffset={6} collisionPadding={8} className="shui-gen-pop" aria-label="Generate options">
        <div ref={pageRef} className="shui-gen-page" data-page={page} aria-busy={saving || undefined}>
          {page === 'main' ? mainPage : page === 'model' ? modelPage : thinkingPage}
          {error ? (
            <p className="shui-gen-note" data-tone="alert" role="alert">
              Couldn’t save: {error}
            </p>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  )
}
