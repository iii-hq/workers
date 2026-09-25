import {
  Button,
  type ComposerControlProps,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  type ExtensionIii,
  IconButton,
  List,
  ListGroupLabel,
  ListItem,
  SearchField,
  StatusBar,
  WorkerConfigurationDialog,
} from '@iii-dev/console-ui'
import { Check, Info, Plus } from 'lucide-react'
import { type KeyboardEvent, useCallback, useEffect, useRef, useState } from 'react'
import { BUILT_IN_PROVIDER, listProviders, type RegisteredProvider } from '../configuration'
import { AddJudgePanel, type AddState } from './add'

/**
 * Session metadata key the harness reads at the start of every turn and
 * stamps on the turn's context (`iii.judge.provider` baggage), so every judge
 * call that turn causes routes to it. Absent = the judge settings' default.
 */
export const SESSION_PROVIDER_KEY = 'judge_provider'
type Engine = Pick<ExtensionIii, 'trigger'>
/** How often an open picker checks whether an added judge registered. */
const ADD_POLL_MS = 3_000
/** An added judge that has not registered by then is reported as stuck. */
const ADD_GIVE_UP_MS = 10 * 60_000

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** The hub's configuration entry and the default provider it stores. */
interface JudgeSettings {
  configurationId: string
  provider: string
}

async function readJudgeSettings(iii: Engine): Promise<JudgeSettings | null> {
  const identity = await iii.trigger<{ id?: unknown }>('judge::configuration-id', {}, { timeoutMs: 5_000 })
  const configurationId = identity?.id
  if (typeof configurationId !== 'string' || !configurationId) return null
  const entry = await iii.trigger<{ value?: { provider?: unknown } }>(
    'configuration::get',
    { id: configurationId },
    { timeoutMs: 5_000 },
  )
  const provider = entry?.value?.provider
  return { configurationId, provider: typeof provider === 'string' && provider ? provider : BUILT_IN_PROVIDER }
}

/** What the judge is and where a session's choice takes effect. */
function JudgeHelp() {
  return (
    <span className="judge-ui-session-help">
      <strong className="judge-ui-session-help-title">Judge</strong>
      <span>
        A fast evaluation model for typed questions: yes or no, pick one, or a score. The agent asks it instead of
        spending a chat-model call. In this session it:
      </span>
      <span className="judge-ui-session-help-list">
        <span>ranks functions and skills when the agent searches the catalog</span>
        <span>checks a function call’s arguments against its schema before it runs</span>
        <span>
          chooses each step of <code>browser::run</code>
        </span>
      </span>
      <span className="judge-ui-session-help-foot">
        A change applies from the next turn. Default follows Settings → Workers → judge.
      </span>
    </span>
  )
}

/** Bind the composer control to the console's engine client once. */
export function createJudgeSessionControl(iii: Engine) {
  return function JudgeSessionControl(props: ComposerControlProps) {
    return <JudgeSessionPicker {...props} iii={iii} />
  }
}

/**
 * The judge provider for THIS session, beside the model picker and laid out
 * like it: a filter, the choices under a heading with Configure, and a quiet
 * footer. Like the model, a change applies from the session's next turn and
 * never touches other sessions or the judge's default.
 */
export function JudgeSessionPicker({ iii, metadata, setMetadata }: ComposerControlProps & { iii: Engine }) {
  const raw = metadata[SESSION_PROVIDER_KEY]
  const stored = typeof raw === 'string' && raw ? raw : undefined
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  // null = not listed yet; both load when the menu opens.
  const [providers, setProviders] = useState<RegisteredProvider[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [settings, setSettings] = useState<JudgeSettings | null>(null)
  const [configuring, setConfiguring] = useState<string | null>(null)
  const [page, setPage] = useState<'judges' | 'add'>('judges')
  // Kept across pages and reopenings: a compose::add outlives the menu.
  const [adds, setAdds] = useState<ReadonlyMap<string, AddState>>(new Map())
  const listRef = useRef<HTMLDivElement>(null)

  // Also settles adds: one is done once its judge registers, stuck after
  // ADD_GIVE_UP_MS.
  const refreshProviders = useCallback(() => {
    listProviders(iii)
      .then((list) => {
        setProviders(list)
        setListError(null)
        const registered = new Set(list.map((entry) => entry.provider))
        const now = Date.now()
        setAdds((current) => {
          const next = new Map(current)
          for (const [worker, add] of current) {
            if (add.kind !== 'adding') continue
            if (registered.has(worker.slice('judge-'.length))) next.set(worker, { kind: 'done' })
            else if (now - add.since > ADD_GIVE_UP_MS)
              next.set(worker, { kind: 'failed', error: 'Not registered after 10 minutes; check Settings → Workers.' })
          }
          return next
        })
      })
      .catch((error: unknown) => {
        setListError(message(error))
        setProviders((current) => current ?? [])
      })
  }, [iii])

  const onOpenChange = (next: boolean) => {
    setOpen(next)
    if (!next) return
    setQuery('')
    setPage('judges')
    setListError(null)
    refreshProviders()
    // Without it the Default row just has no name and Configure hides.
    readJudgeSettings(iii)
      .then(setSettings)
      .catch(() => {})
  }

  const running = new Set((providers ?? []).map((entry) => entry.provider))

  const pending = [...adds.values()].some((add) => add.kind === 'adding')
  useEffect(() => {
    if (!open || !pending) return
    const timer = setInterval(refreshProviders, ADD_POLL_MS)
    return () => clearInterval(timer)
  }, [open, pending, refreshProviders])
  const addJudge = (worker: string) => {
    setAdds((current) => new Map(current).set(worker, { kind: 'adding', since: Date.now() }))
    const fail = (error: string) => setAdds((current) => new Map(current).set(worker, { kind: 'failed', error }))
    iii
      .trigger<{ status?: string; error?: { message?: string } | null }>(
        'compose::add',
        { workers: [worker] },
        { timeoutMs: 600_000 },
      )
      .then((reply) => {
        if (reply?.status === 'failed') fail(reply.error?.message ?? 'compose::add failed')
      })
      .catch((error: unknown) => fail(message(error)))
  }
  const names = [...running]
  if (stored && providers && !running.has(stored)) names.push(stored)
  const needle = query.trim().toLowerCase()
  const matches = names.filter((name) => name.includes(needle))
  const defaultName = settings?.provider
  const showDefault = !needle || 'default'.includes(needle) || (defaultName?.includes(needle) ?? false)

  const choose = (value: string | undefined) => {
    setMetadata({ [SESSION_PROVIDER_KEY]: value })
    setOpen(false)
    // A local provider loads its model on first use: start that now, not on
    // the session's next turn, whose short judge calls would time out on it.
    if (value) {
      iii.trigger('judge::models::list', { provider: value, timeout_ms: 1_000 }, { timeoutMs: 10_000 }).catch(() => {})
    }
  }
  const onFilterKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      listRef.current?.querySelector<HTMLElement>('[data-list-item]:not([disabled])')?.focus()
    } else if (event.key === 'Enter' && needle) {
      event.preventDefault()
      if (matches.length > 0) choose(matches[0])
      else if (showDefault) choose(undefined)
    }
    // The menu's typeahead would pull focus off the field on every letter;
    // Escape still reaches it once the field is empty.
    if (event.key !== 'Escape') event.stopPropagation()
  }
  const selectedMark = <Check size={16} aria-hidden />

  return (
    <span className="judge-ui-session-control">
      <DropdownMenu open={open} onOpenChange={onOpenChange}>
        <DropdownMenuTrigger
          className="judge-ui-session-trigger"
          aria-label={`Judge provider for this session: ${stored ?? 'default'}`}
          title="Judge provider for this session"
        >
          judge · {stored ?? 'default'}
        </DropdownMenuTrigger>
        <DropdownMenuContent side="top" align="end" className="judge-ui-session-menu" aria-label="Judge for this session">
          {/* The menu keeps Tab from moving focus; let it reach Configure, Add and the rows. */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: only lets Tab through to the controls inside */}
          <div className="judge-ui-session-body" onKeyDown={(event) => event.key === 'Tab' && event.stopPropagation()}>
            {page === 'add' ? (
              <AddJudgePanel
                registered={running}
                adds={adds}
                onAdd={addJudge}
                onBack={() => setPage('judges')}
              />
            ) : (
              <div className="judge-ui-session-page" data-page="judges">
                <SearchField
                  className="judge-ui-session-filter"
                  value={query}
                  onChange={setQuery}
                  onKeyDown={onFilterKeyDown}
                  placeholder="Filter judges…"
                  aria-label="Filter judges"
                  // biome-ignore lint/a11y/noAutofocus: the menu opened to pick a judge; typing is the fastest way to one
                  autoFocus
                  autoComplete="off"
                  spellCheck={false}
                />
                <div className="judge-ui-session-heading">
                  <ListGroupLabel>Judge for this session</ListGroupLabel>
                  <span className="judge-ui-session-actions">
                    {settings ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setOpen(false)
                          setConfiguring(settings.configurationId)
                        }}
                      >
                        Configure
                      </Button>
                    ) : null}
                    <IconButton label="Add a judge" variant="ghost" onClick={() => setPage('add')}>
                      <Plus size={16} aria-hidden />
                    </IconButton>
                  </span>
                </div>
                <List ref={listRef} className="judge-ui-session-list" aria-label="Judge providers">
                  {showDefault ? (
                    <ListItem
                      className="judge-ui-session-row"
                      selected={!stored}
                      label="Default"
                      description={defaultName ? `${defaultName}, from judge settings` : 'From judge settings'}
                      trailing={stored ? undefined : selectedMark}
                      onClick={() => choose(undefined)}
                    />
                  ) : null}
                  {providers === null ? (
                    <ListItem className="judge-ui-session-row" disabled label="Checking providers…" />
                  ) : (
                    matches.map((name) => (
                      <ListItem
                        key={name}
                        className="judge-ui-session-row"
                        selected={name === stored}
                        label={name}
                        description={running.has(name) ? undefined : 'Not running'}
                        trailing={name === stored ? selectedMark : undefined}
                        onClick={() => choose(name)}
                      />
                    ))
                  )}
                  {providers !== null && !showDefault && matches.length === 0 ? (
                    <p className="judge-ui-session-note">No judge matches “{query.trim()}”.</p>
                  ) : null}
                  {listError ? (
                    <p className="judge-ui-session-note" role="alert">
                      Could not list judges: {listError}
                    </p>
                  ) : null}
                </List>
                <StatusBar as="footer" className="judge-ui-session-footer">
                  Applies from this session’s next turn
                </StatusBar>
              </div>
            )}
          </div>
        </DropdownMenuContent>
      </DropdownMenu>
      <IconButton
        label="What is the judge?"
        tooltip={<JudgeHelp />}
        tooltipSide="top"
        className="judge-ui-session-help-button"
      >
        <Info size={16} aria-hidden />
      </IconButton>
      <WorkerConfigurationDialog configurationId={configuring} onClose={() => setConfiguring(null)} />
    </span>
  )
}
