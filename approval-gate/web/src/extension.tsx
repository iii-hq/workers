import {
  type ComponentType,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
  Select,
} from './ui'

type ApprovalMode = 'manual' | 'auto' | 'full'
type AccessDuration = 'once' | 'session' | 'always'
type ExtensionContext = Record<string, unknown>

interface ConsoleExtensionDisposable {
  dispose(): void
}

interface ConsoleExtensionSlotContribution {
  id: string
  slot: string
  order?: number
  mount(
    element: HTMLElement,
    context: ExtensionContext,
  ): undefined | (() => void) | ConsoleExtensionDisposable
}

export interface ConsoleExtensionHost {
  apiVersion: number
  extension: {
    id: string
    workerVersion: string
  }
  registerSlot(contribution: ConsoleExtensionSlotContribution): () => void
  trigger<T = unknown>(
    functionId: string,
    payload?: Record<string, unknown>,
  ): Promise<T>
  on(
    functionId: string,
    handler: (payload: unknown) => void | Promise<void>,
  ): () => void
  registerTrigger(input: {
    type: string
    function_id: string
    config: Record<string, unknown>
  }): () => void
  browserId: string
}

interface ApprovalSettings {
  mode: ApprovalMode
  alwaysAllow: string[]
  approvedAlways: string[]
  modeSetAt: number
}

interface SessionSnapshot {
  loaded: boolean
  loading: boolean
  settings: ApprovalSettings
}

const modeOptions: Array<{
  value: ApprovalMode
  label: string
  title: string
}> = [
  {
    value: 'manual',
    label: 'manual',
    title: 'pause every function until you approve or deny it',
  },
  {
    value: 'auto',
    label: 'auto',
    title: 'automatically run functions on the configured allowlist',
  },
  {
    value: 'full',
    label: 'full',
    title: 'run every function without asking',
  },
]

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function coerceStringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((entry): entry is string => typeof entry === 'string')
    : []
}

function coerceSettings(raw: unknown): ApprovalSettings {
  const outer = isRecord(raw) ? raw : {}
  const value = isRecord(outer.settings) ? outer.settings : outer
  const mode = value.mode
  return {
    mode: mode === 'auto' || mode === 'full' ? mode : 'manual',
    alwaysAllow: coerceStringArray(value.always_allow),
    approvedAlways: coerceStringArray(value.approved_always),
    modeSetAt: typeof value.mode_set_at === 'number' ? value.mode_set_at : 0,
  }
}

class SessionStore {
  private listeners = new Set<() => void>()
  private snapshot: SessionSnapshot = {
    loaded: false,
    loading: false,
    settings: coerceSettings(null),
  }

  constructor(
    private readonly host: ConsoleExtensionHost,
    private readonly sessionId: string,
  ) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getSnapshot = () => this.snapshot

  private update(next: SessionSnapshot) {
    this.snapshot = next
    for (const listener of this.listeners) listener()
  }

  async load() {
    if (this.snapshot.loaded || this.snapshot.loading) return
    this.update({ ...this.snapshot, loading: true })
    try {
      const settings = await this.host.trigger('approval::get-settings', {
        session_id: this.sessionId,
      })
      this.update({
        loaded: true,
        loading: false,
        settings: coerceSettings(settings),
      })
    } catch (error) {
      console.error('[approval-extension] get-settings failed', error)
      this.update({ ...this.snapshot, loaded: true, loading: false })
    }
  }

  async setMode(mode: ApprovalMode) {
    const previous = this.snapshot.settings
    this.update({
      ...this.snapshot,
      settings: { ...previous, mode, modeSetAt: Date.now() },
    })
    try {
      const settings = await this.host.trigger('approval::set-mode', {
        session_id: this.sessionId,
        mode,
      })
      this.update({ ...this.snapshot, settings: coerceSettings(settings) })
    } catch (error) {
      console.error('[approval-extension] set-mode failed', error)
      this.update({ ...this.snapshot, settings: previous })
    }
  }
}

const stores = new Map<string, SessionStore>()
let rootSequence = 0

function getSessionStore(host: ConsoleExtensionHost, sessionId: string) {
  const existing = stores.get(sessionId)
  if (existing) return existing
  const store = new SessionStore(host, sessionId)
  stores.set(sessionId, store)
  return store
}

function useSessionSettings(host: ConsoleExtensionHost, sessionId: string) {
  const store = useMemo(
    () => getSessionStore(host, sessionId),
    [host, sessionId],
  )
  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  )
  useEffect(() => {
    void store.load()
  }, [store])
  return { store, snapshot }
}

function PermissionSelect({
  value,
  disabled,
  onChange,
}: {
  value: ApprovalMode
  disabled?: boolean
  onChange: (mode: ApprovalMode) => void | Promise<void>
}) {
  return (
    <Select
      value={value}
      options={modeOptions}
      disabled={disabled}
      aria-label="approval mode"
      onChange={(mode) => {
        if (
          mode === 'full' &&
          !window.confirm(
            'Enable full permissions? The agent will run every function without asking, including shell commands and file writes.',
          )
        ) {
          return
        }
        void onChange(mode)
      }}
    />
  )
}

interface SlotProps {
  host: ConsoleExtensionHost
  context: ExtensionContext
}

function ComposerMode({ host, context }: SlotProps) {
  const sessionId =
    typeof context.sessionId === 'string' ? context.sessionId : null
  if (!sessionId) return null
  return <SessionMode host={host} sessionId={sessionId} context={context} />
}

function SessionMode({
  host,
  sessionId,
  context,
}: {
  host: ConsoleExtensionHost
  sessionId: string
  context: ExtensionContext
}) {
  const { store, snapshot } = useSessionSettings(host, sessionId)
  return (
    <PermissionSelect
      value={snapshot.settings.mode}
      disabled={Boolean(context.disabled) || !snapshot.loaded}
      onChange={(mode) => store.setMode(mode)}
    />
  )
}

function FullPermissionsBanner({ host, context }: SlotProps) {
  const sessionId =
    typeof context.sessionId === 'string' ? context.sessionId : null
  if (!sessionId) return null
  return <SessionBanner host={host} sessionId={sessionId} />
}

function SessionBanner({
  host,
  sessionId,
}: {
  host: ConsoleExtensionHost
  sessionId: string
}) {
  const { store, snapshot } = useSessionSettings(host, sessionId)
  if (snapshot.settings.mode !== 'full') return null
  return (
    <div
      role="status"
      className="flex items-center justify-between gap-3 border-alert border-y bg-alert/10 px-4 py-2 text-[12px]"
    >
      <p className="m-0 text-ink-faint">
        <strong className="text-alert">full permissions active</strong> the
        agent runs every function without asking — including writing files,
        executing shells, and sending messages.
      </p>
      <Button onClick={() => void store.setMode('manual')}>disable</Button>
    </div>
  )
}

const destructivePattern =
  /(write|delete|remove|exec|run|send|credential|secret|chmod|move|rename)/i

function PendingActions({ host, context }: SlotProps) {
  const message = isRecord(context.message) ? context.message : {}
  const sessionId = message.sessionId
  const functionCallId = message.functionCallId
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  if (typeof sessionId !== 'string' || typeof functionCallId !== 'string') {
    return (
      <div className="border-rule-2 border-t px-3 py-2 text-[11px] text-warn">
        this approval is missing its session or function-call id.
      </div>
    )
  }

  const resolve = async (
    decision: 'allow' | 'deny',
    accessDuration?: AccessDuration,
  ) => {
    if (busy) return
    setBusy(decision === 'deny' ? 'denying…' : 'approving…')
    setError(null)
    try {
      await host.trigger('approval::resolve', {
        session_id: sessionId,
        function_call_id: functionCallId,
        decision,
        ...(accessDuration ? { access_duration: accessDuration } : {}),
      })
      setBusy('saved; waiting for the function to resume…')
    } catch (reason) {
      setBusy(null)
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  const approveAlways = async () => {
    const functionId = String(message.functionId ?? '')
    if (
      destructivePattern.test(functionId) &&
      !window.confirm(
        `Approve ${functionId} for the rest of this conversation without further prompts?`,
      )
    ) {
      return
    }
    if (busy) return
    setBusy('saving…')
    setError(null)
    try {
      await host.trigger('approval::approve-always', {
        session_id: sessionId,
        function_id: functionId,
      })
      await host.trigger('approval::resolve', {
        session_id: sessionId,
        function_call_id: functionCallId,
        decision: 'allow',
      })
      setBusy('saved; waiting for the function to resume…')
    } catch (reason) {
      setBusy(null)
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  const filesystemAccess = isRecord(message.filesystemAccess)
    ? message.filesystemAccess
    : null
  const requestedRoot = filesystemAccess?.requestedRoot

  return (
    <div className="flex flex-col gap-2 border-rule-2 border-t px-3 py-2 text-[12px]">
      {typeof requestedRoot === 'string' ? (
        <>
          <code
            className="overflow-hidden text-ellipsis whitespace-nowrap bg-paper-2 px-2 py-1 text-ink"
            title={requestedRoot}
          >
            {requestedRoot}
          </code>
          <p className="m-0 text-ink-faint">
            the function reached this folder and paused before accessing it.
            choose how long to allow access.
          </p>
          <ActionRow>
            <Button
              disabled={Boolean(busy)}
              onClick={() => void resolve('allow', 'once')}
            >
              allow once
            </Button>
            <Button
              variant="secondary"
              disabled={Boolean(busy)}
              onClick={() => void resolve('allow', 'session')}
            >
              allow this session
            </Button>
            <Button
              variant="secondary"
              disabled={Boolean(busy)}
              onClick={() => {
                if (
                  window.confirm(
                    `Always allow ${requestedRoot}? This adds it to shell fs.host_roots for every conversation.`,
                  )
                ) {
                  void resolve('allow', 'always')
                }
              }}
            >
              always allow…
            </Button>
            <Button
              variant="secondary"
              disabled={Boolean(busy)}
              onClick={() => void resolve('deny')}
            >
              deny
            </Button>
          </ActionRow>
          <Button
            variant="link"
            className="self-start"
            onClick={() =>
              window.dispatchEvent(
                new CustomEvent('approval-gate:open-filesystem-access', {
                  detail: { sessionId },
                }),
              )
            }
          >
            manage filesystem access…
          </Button>
        </>
      ) : (
        <>
          <p className="m-0 text-ink-faint">
            execution is paused until you approve or deny this call.
          </p>
          <ActionRow>
            <Button
              disabled={Boolean(busy)}
              onClick={() => void resolve('allow')}
            >
              approve
            </Button>
            <Button
              variant="secondary"
              disabled={Boolean(busy)}
              onClick={() => void resolve('deny')}
            >
              deny
            </Button>
            <Button
              variant="secondary"
              disabled={Boolean(busy)}
              onClick={() => void approveAlways()}
            >
              approve always
            </Button>
          </ActionRow>
        </>
      )}
      {busy ? <span className="text-[11px] text-ink-faint">{busy}</span> : null}
      {error ? <div className="text-[11px] text-warn">{error}</div> : null}
    </div>
  )
}

function ActionRow({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap items-center gap-2">{children}</div>
}

interface StructuredRule extends Record<string, unknown> {
  action?: unknown
  modes?: unknown
  function?: unknown
}

function structuredRule(entry: unknown): StructuredRule | null {
  return isRecord(entry) ? entry : null
}

function autoAllowlist(rules: unknown): Set<string> {
  return new Set(
    (Array.isArray(rules) ? rules : [])
      .map(structuredRule)
      .filter(
        (rule): rule is StructuredRule =>
          rule !== null &&
          rule.action === 'allow' &&
          Array.isArray(rule.modes) &&
          rule.modes.includes('auto') &&
          typeof rule.function === 'string',
      )
      .map((rule) => rule.function as string),
  )
}

function withoutAutoRules(rules: unknown): unknown[] {
  return (Array.isArray(rules) ? rules : []).filter((entry) => {
    const rule = structuredRule(entry)
    return !(
      rule?.action === 'allow' &&
      Array.isArray(rule.modes) &&
      rule.modes.length === 1 &&
      rule.modes[0] === 'auto'
    )
  })
}

async function readConfiguration(host: ConsoleExtensionHost, id: string) {
  const response = await host.trigger('configuration::get', { id, raw: true })
  return isRecord(response) && isRecord(response.value) ? response.value : {}
}

function SettingsPanel({ host }: SlotProps) {
  const [config, setConfig] = useState<Record<string, unknown> | null>(null)
  const [functions, setFunctions] = useState<string[]>([])
  const [allowlistOpen, setAllowlistOpen] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void Promise.all([
      readConfiguration(host, 'approval-gate'),
      host.trigger<{ functions?: Array<{ function_id?: unknown }> }>(
        'engine::functions::list',
        {},
      ),
    ])
      .then(([nextConfig, catalog]) => {
        if (cancelled) return
        setConfig(nextConfig)
        setFunctions(
          (Array.isArray(catalog.functions) ? catalog.functions : [])
            .map((entry) => entry.function_id)
            .filter(
              (id): id is string =>
                typeof id === 'string' &&
                !id.startsWith('approval::') &&
                !id.startsWith('configuration::'),
            )
            .sort(),
        )
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason))
      })
    return () => {
      cancelled = true
    }
  }, [host])

  const save = useCallback(
    async (defaultMode: ApprovalMode, allowlist: Set<string>) => {
      if (!config) return
      const rules = [
        ...withoutAutoRules(config.rules),
        ...[...allowlist].map((functionId) => ({
          function: functionId,
          action: 'allow',
          modes: ['auto'],
        })),
      ]
      const next = { ...config, default_mode: defaultMode, rules }
      setConfig(next)
      setError(null)
      try {
        await host.trigger('configuration::set', {
          id: 'approval-gate',
          value: next,
        })
      } catch (reason) {
        setConfig(config)
        setError(reason instanceof Error ? reason.message : String(reason))
      }
    },
    [config, host],
  )

  if (error && !config) {
    return <p className="mt-10 text-[12px] text-warn">{error}</p>
  }
  if (!config) {
    return (
      <p className="mt-10 text-[12px] text-ink-faint">loading permissions…</p>
    )
  }

  const defaultMode =
    config.default_mode === 'auto' || config.default_mode === 'full'
      ? config.default_mode
      : 'manual'
  const allowlist = autoAllowlist(config.rules)

  return (
    <div className="font-sans text-ink">
      <SettingsSection
        title="permissions"
        description="defaults stored in the approval-gate configuration entry. applies to new conversations only."
      >
        <SettingsRow
          label="default mode"
          description="manual prompts for everything · auto uses the allowlist · full skips prompts"
        >
          <PermissionSelect
            value={defaultMode}
            onChange={(mode) => save(mode, allowlist)}
          />
        </SettingsRow>
        {defaultMode === 'auto' ? (
          <SettingsRow
            label="allowlist"
            description="functions trusted automatically for new conversations"
          >
            <Button
              variant="secondary"
              onClick={() => setAllowlistOpen((open) => !open)}
            >
              {allowlistOpen
                ? 'close'
                : `manage${allowlist.size ? ` (${allowlist.size})` : ''}`}
            </Button>
          </SettingsRow>
        ) : null}
        {allowlistOpen && defaultMode === 'auto' ? (
          <div className="grid max-h-80 overflow-auto border border-rule-2 p-2">
            {functions.map((functionId) => (
              <label
                key={functionId}
                className="flex items-center gap-2 p-1 text-[12px]"
              >
                <input
                  type="checkbox"
                  checked={allowlist.has(functionId)}
                  onChange={(event) => {
                    const next = new Set(allowlist)
                    if (event.currentTarget.checked) next.add(functionId)
                    else next.delete(functionId)
                    void save(defaultMode, next)
                  }}
                />
                <code>{functionId}</code>
              </label>
            ))}
          </div>
        ) : null}
        {error ? <p className="mt-2 text-[11px] text-warn">{error}</p> : null}
      </SettingsSection>

      <SettingsSection
        title="filesystem access"
        description="the chosen workspace is always available; access outside it is approved from the function card."
      >
        <Button asChild variant="secondary">
          <a href="#/workers/configuration/shell/fs/host_roots">
            edit permanent roots
          </a>
        </Button>
      </SettingsSection>
    </div>
  )
}

function SettingsSection({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children: ReactNode
}) {
  return (
    <section className="mt-10">
      <h2 className="m-0 mb-1 text-[14px] font-normal capitalize tracking-[0.06em]">
        {title}
      </h2>
      <p className="m-0 mb-3 text-[12px] text-ink-faint">{description}</p>
      <div className="border-rule border-t">{children}</div>
    </section>
  )
}

function SettingsRow({
  label,
  description,
  children,
}: {
  label: string
  description: string
  children: ReactNode
}) {
  return (
    <div className="grid grid-cols-[96px_1fr_auto] items-center gap-4 border-rule border-b py-3 text-[13px]">
      <span>{label}</span>
      <small className="overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-ink-faint">
        {description}
      </small>
      {children}
    </div>
  )
}

function WorkspaceAccess({ host, context }: SlotProps) {
  const sessionId =
    typeof context.sessionId === 'string' ? context.sessionId : null
  const [open, setOpen] = useState(false)
  const [loading, setLoading] = useState(false)
  const [grants, setGrants] = useState<string[]>([])
  const [permanent, setPermanent] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!sessionId) return
    const onOpen = (event: Event) => {
      const detail = (event as CustomEvent<unknown>).detail
      if (isRecord(detail) && detail.sessionId === sessionId) setOpen(true)
    }
    window.addEventListener('approval-gate:open-filesystem-access', onOpen)
    return () =>
      window.removeEventListener('approval-gate:open-filesystem-access', onOpen)
  }, [sessionId])

  useEffect(() => {
    if (!open || !sessionId) return
    let cancelled = false
    setLoading(true)
    setError(null)
    void Promise.all([
      host
        .trigger('harness::filesystem::grants', { session_id: sessionId })
        .catch(() => ({ roots: [] })),
      readConfiguration(host, 'shell').catch(
        (): Record<string, unknown> => ({}),
      ),
    ]).then(([grantResponse, shellConfig]) => {
      if (cancelled) return
      const grantRecord = isRecord(grantResponse) ? grantResponse : {}
      const fs = isRecord(shellConfig.fs) ? shellConfig.fs : {}
      setGrants(coerceStringArray(grantRecord.roots))
      setPermanent(coerceStringArray(fs.host_roots))
      setLoading(false)
    })
    return () => {
      cancelled = true
    }
  }, [host, open, sessionId])

  if (!sessionId) return null
  const workspace =
    typeof context.workingDir === 'string' ? [context.workingDir] : []

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="link">access: workspace</Button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>filesystem access</DialogTitle>
        <DialogDescription className="mt-1">
          folders the agent can read and write in this conversation.
        </DialogDescription>
        {context.sessionBusy ? (
          <p className="mt-3 text-[11px] text-ink-faint">
            the agent is running; revoked access may be requested again.
          </p>
        ) : null}
        {loading ? (
          <p className="mt-5 text-[12px] text-ink-faint">
            loading filesystem access…
          </p>
        ) : (
          <div className="my-5 grid gap-5">
            <FolderGroup title="workspace" roots={workspace} />
            <FolderGroup
              title="allowed this session"
              roots={grants}
              onRemove={async (root) => {
                setError(null)
                try {
                  await host.trigger('harness::filesystem::revoke', {
                    session_id: sessionId,
                    root,
                  })
                  setGrants((current) =>
                    current.filter((entry) => entry !== root),
                  )
                } catch (reason) {
                  setError(
                    reason instanceof Error ? reason.message : String(reason),
                  )
                }
              }}
            />
            <FolderGroup title="always allowed" roots={permanent} />
          </div>
        )}
        {error ? <p className="mb-3 text-[11px] text-warn">{error}</p> : null}
        <Button
          variant="link"
          onClick={() => {
            setOpen(false)
            window.location.hash = '/workers/configuration/shell/fs/host_roots'
          }}
        >
          edit permanent roots →
        </Button>
      </DialogContent>
    </Dialog>
  )
}

function FolderGroup({
  title,
  roots,
  onRemove,
}: {
  title: string
  roots: string[]
  onRemove?: (root: string) => void | Promise<void>
}) {
  return (
    <section>
      <h3 className="mb-1 text-[11px] font-normal uppercase tracking-[0.06em] text-ink-faint">
        {title}
      </h3>
      <div className="border border-rule-2">
        {roots.length === 0 ? (
          <span className="flex px-2 py-1.5 text-[12px] text-ink-faint">
            none
          </span>
        ) : null}
        {roots.map((root) => (
          <div
            key={root}
            className="flex items-center justify-between gap-2 px-2 py-1.5 text-[12px]"
          >
            <code
              className="overflow-hidden text-ellipsis whitespace-nowrap"
              title={root}
            >
              {root}
            </code>
            {onRemove ? (
              <Button variant="link" onClick={() => void onRemove(root)}>
                revoke
              </Button>
            ) : null}
          </div>
        ))}
      </div>
    </section>
  )
}

function ContextBridge({
  element,
  initialContext,
  host,
  component: Component,
}: {
  element: HTMLElement
  initialContext: ExtensionContext
  host: ConsoleExtensionHost
  component: ComponentType<SlotProps>
}) {
  const [context, setContext] = useState(initialContext)
  useEffect(() => {
    const onContext = (event: Event) => {
      const detail = (event as CustomEvent<unknown>).detail
      setContext(isRecord(detail) ? detail : {})
    }
    element.addEventListener('iii:console-extension-context', onContext)
    return () =>
      element.removeEventListener('iii:console-extension-context', onContext)
  }, [element])
  return <Component host={host} context={context} />
}

function mountReact(
  host: ConsoleExtensionHost,
  Component: ComponentType<SlotProps>,
  element: HTMLElement,
  context: ExtensionContext,
) {
  element.dataset.consoleExtension = 'approval-gate'
  const root = createRoot(element, {
    identifierPrefix: `approval-gate-${rootSequence++}-`,
  })
  root.render(
    <ContextBridge
      element={element}
      initialContext={context}
      host={host}
      component={Component}
    />,
  )
  return () => {
    root.unmount()
    delete element.dataset.consoleExtension
  }
}

export function activate(host: ConsoleExtensionHost) {
  if (host.apiVersion !== 1) {
    throw new Error(
      `approval-gate requires console extension API v1, got ${host.apiVersion}`,
    )
  }

  const register = (
    id: string,
    slot: string,
    Component: ComponentType<SlotProps>,
  ) =>
    host.registerSlot({
      id,
      slot,
      mount: (element, context) =>
        mountReact(host, Component, element, context),
    })

  const disposers = [
    register(
      'approval-gate.composer-mode',
      'chat.composer.controls',
      ComposerMode,
    ),
    register('approval-gate.full-banner', 'chat.banner', FullPermissionsBanner),
    register(
      'approval-gate.pending-actions',
      'function-call.pending-actions',
      PendingActions,
    ),
    register('approval-gate.settings', 'settings.sections', SettingsPanel),
    register(
      'approval-gate.workspace-access',
      'chat.workspace-access',
      WorkspaceAccess,
    ),
  ]

  return {
    dispose() {
      for (const dispose of disposers.reverse()) dispose()
      stores.clear()
    },
  }
}
