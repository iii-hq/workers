import {
  Button,
  type ComposerControlProps,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  IconButton,
  StatusBar,
  uiClasses,
  WorkerConfigurationDialog,
  type WorkerConfigurationPanelProps,
} from '@iii-dev/console-ui'
import { ArrowLeft, Info } from 'lucide-react'
import {
  type ComponentType,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'
import { AddJudgePanel, type AddState } from './add'
import {
  ADD_GIVE_UP_MS,
  addWorker,
  type Engine,
  type JudgeProvider,
  type JudgeSettings,
  listJudges,
  message,
  readJudgeSettings,
} from './engine'
import { type ConfigureTarget, JudgesPanel, providerTarget } from './judges'

/**
 * Session metadata key the harness reads at the start of every turn and
 * stamps on the turn's context (`iii.judge.provider` baggage), so every judge
 * call that turn causes routes to it. Absent = the judge settings' default.
 */
export const SESSION_PROVIDER_KEY = 'judge_provider'
/** The shared picker-page motion; its CSS ships with every console. */
const PICKER_PAGE = uiClasses.motionPickerPage ?? 'iii-ui-motion-picker-page'
/** How long a page that is leaving stays rendered: the slide out. */
const PAGE_TRANSITION_MS = 250
/** A burst of engine changes (a worker registering its functions) reads once. */
const LIVE_DEBOUNCE_MS = 300
/** The hub's and every provider's configuration entry. */
const JUDGE_CONFIGURATION = /^judge(-[a-z0-9-]{1,64})?$/

/** Unique per open menu: two pickers never share a handler. */
let liveSeq = 0

type Page = 'judges' | 'add' | 'configure'

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
        A change applies from the next turn. Default follows the judge settings.
      </span>
    </span>
  )
}

/**
 * Bind the composer control to the console's engine client once.
 * `configurationPanel` is the Console's inline settings editor
 * (`host.components.WorkerConfigurationPanel`); without it, Configure opens
 * the worker in Settings.
 */
export function createJudgeSessionControl(
  iii: Engine,
  configurationPanel?: ComponentType<WorkerConfigurationPanelProps>,
) {
  return function JudgeSessionControl(props: ComposerControlProps) {
    return <JudgeSessionPicker {...props} iii={iii} configurationPanel={configurationPanel} />
  }
}

function SubpageHeader({
  title,
  description,
  onBack,
  backRef,
}: {
  title: string
  description: string
  onBack(): void
  backRef: RefObject<HTMLButtonElement | null>
}) {
  return (
    <div className="judge-ui-session-subheader">
      <IconButton ref={backRef} label="Back to judges" tooltip={false} variant="ghost" onClick={onBack}>
        <ArrowLeft size={16} aria-hidden />
      </IconButton>
      <div className="judge-ui-session-subheader-copy">
        <h2 className="judge-ui-session-subtitle">{title}</h2>
        <p className="judge-ui-session-subdescription">{description}</p>
      </div>
    </div>
  )
}

function PickerPage({ page, active, children }: { page: Page; active: boolean; children: ReactNode }) {
  return (
    <div
      className={`judge-ui-session-page ${PICKER_PAGE}`}
      data-page={page}
      data-active={active}
      aria-hidden={!active}
      inert={!active}
    >
      {children}
    </div>
  )
}

/**
 * The judge provider for THIS session, beside the model picker and built
 * like it: a filter, a rail of judges with the add button, the choices
 * grouped with a Configure each, and pages that slide in for adding a judge
 * from the registry and for a judge's settings, edited right here. Like the
 * model, a choice applies from the session's next turn and never touches
 * other sessions or the judge's default.
 */
export function JudgeSessionPicker({
  iii,
  metadata,
  setMetadata,
  configurationPanel: ConfigurationPanel,
}: ComposerControlProps & { iii: Engine; configurationPanel?: ComponentType<WorkerConfigurationPanelProps> }) {
  const raw = metadata[SESSION_PROVIDER_KEY]
  const stored = typeof raw === 'string' && raw ? raw : undefined
  const [open, setOpen] = useState(false)
  const [page, setPage] = useState<Page>('judges')
  // The settings page keeps its content while it slides out.
  const [configuring, setConfiguring] = useState<ConfigureTarget | null>(null)
  const [shownConfiguring, setShownConfiguring] = useState<ConfigureTarget | null>(null)
  const [addShown, setAddShown] = useState(false)
  // The settings page has edits not saved; leaving asks first.
  const [dirty, setDirty] = useState(false)
  const [leaving, setLeaving] = useState<{ run(): void } | null>(null)
  // Consoles without the inline editor open the worker in Settings.
  const [settingsDialog, setSettingsDialog] = useState<string | null>(null)
  // null = not listed yet; both load when the menu opens.
  const [providers, setProviders] = useState<JudgeProvider[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [settings, setSettings] = useState<JudgeSettings | null>(null)
  // Kept across pages and reopenings: a compose::add outlives the menu.
  const [adds, setAdds] = useState<ReadonlyMap<string, AddState>>(new Map())
  const addBackRef = useRef<HTMLButtonElement>(null)
  const configureBackRef = useRef<HTMLButtonElement>(null)
  const returnFocusRef = useRef<HTMLElement | null>(null)

  // Also settles adds: one is done once its judge registers (read again on
  // the engine's own change events), stuck after ADD_GIVE_UP_MS.
  const refresh = useCallback(() => {
    // Without the settings the Default row has no name and no Configure.
    readJudgeSettings(iii)
      .then(setSettings)
      .catch(() => {})
    listJudges(iii)
      .then((list) => {
        setProviders(list)
        setListError(null)
        const registered = new Set(list.map((entry) => entry.provider))
        const now = Date.now()
        setAdds((current) => {
          const next = new Map(current)
          for (const [worker, add] of current) {
            if (add.kind === 'done') continue
            // A judge that registers is added, even after an add reported failure.
            if (registered.has(worker.slice('judge-'.length))) next.set(worker, { kind: 'done' })
            else if (add.kind === 'adding' && now - add.since > ADD_GIVE_UP_MS)
              next.set(worker, {
                kind: 'failed',
                error: 'Not registered after 10 minutes; check Settings → Workers.',
                at: now,
              })
          }
          return next
        })
      })
      .catch((error: unknown) => {
        setListError(message(error))
        setProviders((current) => current ?? [])
      })
  }, [iii])

  // Configuration entries this picker reads, beyond the `judge-*` names.
  const watchedIds = useRef(new Set<string>())
  watchedIds.current = new Set(
    [settings?.configurationId, ...(providers ?? []).map((entry) => entry.configurationId)].filter(
      (id): id is string => typeof id === 'string',
    ),
  )

  // While the menu is open the list follows the engine, never a timer: a
  // judge registering or going away (an add landing, late or not) fires
  // `engine::functions-available`, a settings change the `configuration`
  // trigger. The configuration binding names no id: an id-scoped binding
  // holds the entry, and dropping it can start the entry's expiry.
  useEffect(() => {
    if (!open) return
    const seq = ++liveSeq
    const functionsHandler = `iii::judge-ui::session::functions-${seq}`
    const configurationHandler = `iii::judge-ui::session::configuration-${seq}`
    let timer: ReturnType<typeof setTimeout> | null = null
    const schedule = () => {
      if (timer !== null) clearTimeout(timer)
      timer = setTimeout(() => {
        timer = null
        refresh()
      }, LIVE_DEBOUNCE_MS)
    }
    const offHandlers = [
      iii.on(functionsHandler, schedule),
      iii.on<{ id?: unknown }>(configurationHandler, (event) => {
        const id = typeof event?.id === 'string' ? event.id : null
        if (id !== null && (JUDGE_CONFIGURATION.test(id) || watchedIds.current.has(id))) schedule()
      }),
    ]
    const offTriggers = [
      { type: 'engine::functions-available', handler: functionsHandler },
      { type: 'configuration', handler: configurationHandler },
    ].flatMap(({ type, handler }) => {
      try {
        return [iii.registerTrigger({ type, function_id: `${handler}::${iii.browserId}`, config: {} })]
      } catch {
        // A trigger type missing on this engine: the menu reads on open.
        return []
      }
    })
    return () => {
      if (timer !== null) clearTimeout(timer)
      for (const off of [...offTriggers, ...offHandlers]) {
        try {
          off()
        } catch {
          // already gone
        }
      }
    }
  }, [open, iii, refresh])

  // A page that left keeps its content until it has slid out.
  useEffect(() => {
    if (configuring || !shownConfiguring) return
    const timer = setTimeout(() => setShownConfiguring(null), PAGE_TRANSITION_MS)
    return () => clearTimeout(timer)
  }, [configuring, shownConfiguring])
  useEffect(() => {
    if (page === 'add' || !addShown) return
    const timer = setTimeout(() => setAddShown(false), PAGE_TRANSITION_MS)
    return () => clearTimeout(timer)
  }, [page, addShown])

  // Focus follows the page: a subpage starts on its back button, and coming
  // back returns to the control that left.
  useEffect(() => {
    if (!open) return
    if (page === 'judges') {
      const origin = returnFocusRef.current
      returnFocusRef.current = null
      origin?.focus()
      return
    }
    const back = (page === 'add' ? addBackRef : configureBackRef).current
    back?.focus()
    const frame = requestAnimationFrame(() => back?.focus())
    return () => cancelAnimationFrame(frame)
  }, [open, page])

  /** Run `action`, after the operator agrees to drop unsaved settings. */
  const guard = (action: () => void) => {
    if (page === 'configure' && dirty) setLeaving({ run: action })
    else action()
  }
  const remember = () => {
    returnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
  }
  const leaveSubpage = () => {
    setLeaving(null)
    setDirty(false)
    setConfiguring(null)
    setPage('judges')
  }

  const onOpenChange = (next: boolean) => {
    if (next) {
      setOpen(true)
      setListError(null)
      refresh()
      return
    }
    guard(() => {
      leaveSubpage()
      returnFocusRef.current = null
      setOpen(false)
    })
  }

  const configure = (target: ConfigureTarget) => {
    if (!ConfigurationPanel) {
      setOpen(false)
      setSettingsDialog(target.configurationId)
      return
    }
    if (page === 'judges') remember()
    setLeaving(null)
    setDirty(false)
    setConfiguring(target)
    setShownConfiguring(target)
    setPage('configure')
  }
  const openAdd = () => {
    remember()
    setAddShown(true)
    setPage('add')
  }

  const choose = (value: string | undefined) => {
    setMetadata({ [SESSION_PROVIDER_KEY]: value })
    setOpen(false)
    // A local provider loads its model on first use: start that now, not on
    // the session's next turn, whose short judge calls would time out on it.
    if (value) {
      iii.trigger('judge::models::list', { provider: value, timeout_ms: 1_000 }, { timeoutMs: 10_000 }).catch(() => {})
    }
  }

  const addJudge = (worker: string) => {
    setAdds((current) => new Map(current).set(worker, { kind: 'adding', since: Date.now() }))
    // Only an add still in progress fails: a registered judge stays added.
    addWorker(iii, worker, (error) =>
      setAdds((current) =>
        current.get(worker)?.kind === 'adding'
          ? new Map(current).set(worker, { kind: 'failed', error, at: Date.now() })
          : current,
      ),
    )
  }

  const registered = new Set((providers ?? []).map((entry) => entry.provider))
  const targets = new Map(
    (providers ?? []).flatMap((entry) => {
      const target = providerTarget(entry)
      return target ? [[entry.provider, target] as const] : []
    }),
  )

  // The menu keeps Tab and typed keys for its own navigation; the pages are
  // forms and lists, so their keys stay theirs. Escape still closes.
  const keepKeys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'Escape') event.stopPropagation()
  }

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
          {/* biome-ignore lint/a11y/noStaticElementInteractions: only keeps keys from the menu's own navigation */}
          <div className="judge-ui-session-pages" onKeyDown={keepKeys}>
            <PickerPage page="judges" active={page === 'judges'}>
              <JudgesPanel
                providers={providers}
                settings={settings}
                stored={stored}
                listError={listError}
                onChoose={choose}
                onConfigure={configure}
                onAdd={openAdd}
              />
              <StatusBar as="footer" className="judge-ui-session-footer">
                Applies from this session’s next turn
              </StatusBar>
            </PickerPage>
            <PickerPage page="add" active={page === 'add'}>
              <SubpageHeader
                title="Add a judge"
                description="Judge workers from the workers registry."
                backRef={addBackRef}
                onBack={() => setPage('judges')}
              />
              {addShown ? (
                <AddJudgePanel
                  registered={registered}
                  adds={adds}
                  targets={targets}
                  onAdd={addJudge}
                  onConfigure={configure}
                />
              ) : null}
            </PickerPage>
            <PickerPage page="configure" active={page === 'configure'}>
              {shownConfiguring ? (
                <>
                  <SubpageHeader
                    title={shownConfiguring.title}
                    description={shownConfiguring.description}
                    backRef={configureBackRef}
                    onBack={() => guard(leaveSubpage)}
                  />
                  {leaving ? (
                    <div className="judge-ui-session-discard" role="alert">
                      <span>Discard the changes you have not saved?</span>
                      <span className="judge-ui-session-discard-actions">
                        <Button variant="ghost" size="sm" onClick={() => setLeaving(null)}>
                          Keep editing
                        </Button>
                        <Button variant="pill" size="sm" onClick={() => leaving.run()}>
                          Discard
                        </Button>
                      </span>
                    </div>
                  ) : null}
                  {ConfigurationPanel ? (
                    <ConfigurationPanel
                      key={shownConfiguring.configurationId}
                      configurationId={shownConfiguring.configurationId}
                      onDirtyChange={setDirty}
                      onSaved={refresh}
                      className="judge-ui-session-settings"
                    />
                  ) : null}
                </>
              ) : null}
            </PickerPage>
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
      <WorkerConfigurationDialog configurationId={settingsDialog} onClose={() => setSettingsDialog(null)} />
    </span>
  )
}
