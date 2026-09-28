/**
 * Webhooks view: the PR webhook setup checklist served by
 * `github::setup::webhooks-status`. Each prerequisite turns green only once
 * verified, in order: the quick-tunnel worker, the cloudflared binary it runs,
 * then the http worker's restricted webhook listener. "Enable webhooks"
 * unlocks when all three are green.
 *
 * Nothing downloads cloudflared: its row links Cloudflare's install page and
 * re-checks on demand. Installing the quick-tunnel worker runs `compose::add`
 * only after an explicit confirmation. Enabling writes `webhooks.enabled` and
 * restarts the github worker (asked first), because webhook storage opens at
 * startup. A late response never overwrites a newer one.
 */

import {
  Badge,
  Button,
  EmptyState,
  type Host,
  type LiveAnnouncement,
  LiveRegion,
  PageMain,
  SettingsList,
  SettingsRow,
  SettingsSection,
  uiClasses,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ExternalLink, RefreshCw, Webhook } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'

type CheckState = 'ok' | 'missing' | 'blocked' | 'unknown'

type SetupFix =
  | { kind: 'install_worker'; worker: string; command: string }
  | { kind: 'update_worker'; worker: string; command: string }
  | { kind: 'open_url'; url: string }
  | { kind: 'enable_http_listener'; port: number }

interface SetupCheck {
  id: string
  title: string
  state: CheckState
  detail: string
  fix?: SetupFix
}

export interface SetupStatus {
  enabled: boolean
  active: boolean
  ready: boolean
  restart_required: boolean
  checks: SetupCheck[]
  storage_error?: string
}

const STATUS_FN = 'github::setup::webhooks-status'
const LISTENER_FN = 'github::setup::enable-http-listener'
const ENABLE_FN = 'github::setup::enable-webhooks'
/** Webhook storage opens at startup: the github compose container restarts. */
const GITHUB_CONTAINER = 'github'

const BADGES: Record<CheckState, { variant: 'ok' | 'warn' | 'alert' | 'default'; label: string }> = {
  ok: { variant: 'ok', label: 'Ready' },
  missing: { variant: 'alert', label: 'Missing' },
  blocked: { variant: 'default', label: 'Waiting' },
  unknown: { variant: 'warn', label: 'Unverified' },
}

/** Keep "Checking…" on screen long enough to be seen when the check is fast. */
const MIN_CHECKING_MS = 500
/** compose::add only admits the operation; its outcome is polled this often. */
const OPERATION_POLL_MS = 1_000
/** Give up waiting on an install after this long (it keeps running in compose). */
const OPERATION_TIMEOUT_MS = 600_000

interface OperationSnapshot {
  operation_id?: string
  status?: string
  last_event?: { detail?: string; phase?: string }
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/** Throw on a compose lifecycle result that resolved but reports a failure. */
function assertComposeOk(action: string, result: unknown) {
  if (!result || typeof result !== 'object') return
  const r = result as {
    error?: unknown
    status?: unknown
    containers?: { container?: string; error?: unknown }[]
  }
  const errors: string[] = []
  if (typeof r.error === 'string' && r.error) errors.push(r.error)
  if (typeof r.status === 'string' && ['failed', 'error', 'cancelled'].includes(r.status)) {
    errors.push(`status ${r.status}`)
  }
  for (const c of r.containers ?? []) {
    if (typeof c.error === 'string' && c.error) errors.push(`${c.container ?? 'container'}: ${c.error}`)
  }
  if (errors.length > 0) throw new Error(`${action} failed: ${errors.join('; ')}`)
}

function summarize(status: SetupStatus): string {
  const total = status.checks.length
  const open = status.checks.filter((check) => check.state !== 'ok').length
  return open === 0
    ? `All ${total} prerequisites ready.`
    : `${open} of ${total} prerequisites need attention.`
}

export function WebhookSetup({ host }: { host: Host }) {
  const [status, setStatus] = useState<SetupStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  /** An action's failure; only the next action or Dismiss clears it. */
  const [actionError, setActionError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [checking, setChecking] = useState(false)
  const [checkedAt, setCheckedAt] = useState<Date | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const { confirm, dialog } = useConfirm()
  const latest = useRef(0)
  const announced = useRef(0)
  /** Stops an install wait when the page unmounts. */
  const mounted = useRef(true)
  useEffect(
    () => () => {
      mounted.current = false
    },
    [],
  )

  /** `announce` reads the result aloud: only for checks the user asked for. */
  const refresh = useCallback(
    async (announce = false) => {
      const request = ++latest.current
      setChecking(true)
      const started = Date.now()
      try {
        const next = await host.iii.trigger<SetupStatus>(STATUS_FN, {}, { timeoutMs: 20_000 })
        await new Promise((resolve) =>
          setTimeout(resolve, Math.max(0, MIN_CHECKING_MS - (Date.now() - started))),
        )
        if (request !== latest.current) return
        setStatus(next)
        setError(null)
        setCheckedAt(new Date())
        if (announce) {
          setAnnouncement({ seq: ++announced.current, text: summarize(next), urgency: 'polite' })
        }
      } catch (err) {
        if (request !== latest.current) return
        const message = errorMessage(err)
        setError(message)
        if (announce) {
          setAnnouncement({
            seq: ++announced.current,
            text: `Check failed: ${message}`,
            urgency: 'assertive',
          })
        }
      } finally {
        if (request === latest.current) setChecking(false)
      }
    },
    [host],
  )

  useEffect(() => {
    void refresh()
  }, [refresh])

  const run = useCallback(
    async (key: string, action: () => Promise<unknown>) => {
      setBusy(key)
      setActionError(null)
      let failed = false
      try {
        await action()
      } catch (err) {
        failed = true
        const message = errorMessage(err)
        setActionError(message)
        setAnnouncement({ seq: ++announced.current, text: message, urgency: 'assertive' })
      } finally {
        setBusy(null)
        // Re-check either way, but never let the summary drown out a failure.
        await refresh(!failed)
      }
    },
    [refresh],
  )

  const installWorker = async (worker: string) => {
    const ok = await confirm({
      title: `Install the ${worker} worker?`,
      description: `Adds ${worker} to this project with compose::add and starts it. It runs cloudflared, which you install yourself.`,
      confirmLabel: 'Install',
    })
    if (ok) await run(`install:${worker}`, () => installAndWait(worker))
  }

  /** compose::add is asynchronous: follow its operation to a terminal status. */
  const installAndWait = async (worker: string) => {
    const operationId = `github-install-${worker}-${crypto.randomUUID().slice(0, 8)}`
    const admitted = await host.iii.trigger(
      'compose::add',
      { workers: [worker], operation_id: operationId },
      { timeoutMs: 60_000 },
    )
    assertComposeOk(`Installing ${worker}`, admitted)
    const deadline = Date.now() + OPERATION_TIMEOUT_MS
    while (mounted.current) {
      const snapshot = await host.iii.trigger<OperationSnapshot>(
        'compose::operation',
        { operation_id: operationId },
        { timeoutMs: 15_000 },
      )
      if (snapshot?.status === 'succeeded') return
      if (snapshot?.status === 'failed' || snapshot?.status === 'cancelled') {
        const detail = snapshot.last_event?.detail
        throw new Error(`Installing ${worker} ${snapshot.status}${detail ? `: ${detail}` : ''}`)
      }
      if (Date.now() > deadline) {
        throw new Error(
          `Installing ${worker} is still running after 10 minutes; check compose logs.`,
        )
      }
      await sleep(OPERATION_POLL_MS)
    }
  }

  const setEnabled = async (enabled: boolean) => {
    const ok = await confirm({
      title: enabled ? 'Enable PR webhooks?' : 'Disable PR webhooks?',
      description:
        'The github worker restarts to apply this; github calls in flight are interrupted.',
      confirmLabel: enabled ? 'Enable and restart' : 'Disable and restart',
      tone: enabled ? 'default' : 'danger',
    })
    if (!ok) return
    await run('enable', async () => {
      const next = await host.iii.trigger<SetupStatus>(ENABLE_FN, { enabled }, { timeoutMs: 30_000 })
      if (next.restart_required) await restartGithub()
    })
  }

  /** Unconfirmed: callers confirm first (Enable/Disable or restartConfirmed). */
  const restartGithub = async () => {
    const result = await host.iii.trigger(
      'compose::restart',
      { container: GITHUB_CONTAINER },
      { timeoutMs: 180_000 },
    )
    assertComposeOk('Restarting github', result)
  }

  const restartConfirmed = async () => {
    const ok = await confirm({
      title: 'Restart the github worker?',
      description: 'Applies the pending webhook setting; github calls in flight are interrupted.',
      confirmLabel: 'Restart',
    })
    if (ok) await run('restart', restartGithub)
  }

  const fixControl = (check: SetupCheck) => {
    const fix = check.fix
    if (check.state === 'ok' || !fix) return undefined
    switch (fix.kind) {
      case 'install_worker':
        return (
          <Button
            size="sm"
            variant="primary"
            disabled={busy !== null}
            onClick={() => void installWorker(fix.worker)}
          >
            {busy === `install:${fix.worker}` ? 'Installing…' : `Install ${fix.worker}`}
          </Button>
        )
      case 'open_url':
        return (
          <Button asChild size="sm" variant="ghost">
            <a href={fix.url} target="_blank" rel="noreferrer">
              Install guide <ExternalLink aria-hidden />
            </a>
          </Button>
        )
      case 'enable_http_listener':
        return (
          <Button
            size="sm"
            variant="primary"
            disabled={busy !== null}
            onClick={() =>
              void run('listener', () =>
                host.iii.trigger(LISTENER_FN, { port: fix.port }, { timeoutMs: 30_000 }),
              )
            }
          >
            {busy === 'listener' ? 'Turning on…' : `Turn on (port ${fix.port})`}
          </Button>
        )
      case 'update_worker':
        return <code className="gh-ui-webhooks__command">{fix.command}</code>
    }
  }

  if (!status) {
    return (
      <PageMain className="gh-ui-main gh-ui-webhooks">
        {error ? (
          <EmptyState
            icon={Webhook}
            title="Webhook setup unavailable"
            description={error}
            action={{ label: 'Try again', onClick: () => void refresh(true) }}
          />
        ) : (
          <p className="gh-ui-webhooks__loading" role="status">
            Checking webhook prerequisites…
          </p>
        )}
      </PageMain>
    )
  }

  const stateLine =
    status.enabled && status.active
      ? 'Active: watched pull requests receive GitHub deliveries.'
      : status.enabled && status.storage_error
        ? `Enabled, but webhook storage failed to open: ${status.storage_error}. Fix it, then restart github.`
        : status.enabled
          ? 'Enabled in configuration; restart github to apply it.'
          : status.active
            ? 'Disabled in configuration; restart github to apply it. Deliveries continue until then.'
            : status.ready
              ? 'Every prerequisite is ready.'
              : 'Complete the steps above to enable PR webhooks.'
  const badge = status.restart_required
    ? { variant: 'warn' as const, label: 'Restart pending' }
    : status.active
      ? { variant: 'ok' as const, label: 'On' }
      : { variant: 'default' as const, label: 'Off' }

  return (
    <PageMain className="gh-ui-main gh-ui-webhooks">
      {dialog}
      <LiveRegion announcement={announcement} />
      <SettingsSection
        title="PR webhook setup"
        description="github::pr::watch needs these on this machine. Nothing is installed without asking, and cloudflared is never downloaded for you."
        action={
          <Button
            size="sm"
            variant="ghost"
            disabled={busy !== null || checking}
            aria-busy={checking}
            onClick={() => void refresh(true)}
          >
            <RefreshCw aria-hidden className={checking ? uiClasses.spin : undefined} />
            {checking ? 'Checking…' : 'Check again'}
          </Button>
        }
      >
        <SettingsList>
          {status.checks.map((check, index) => (
            <SettingsRow
              key={check.id}
              layout="auto"
              label={`${index + 1}. ${check.title}`}
              description={check.detail}
              meta={<Badge variant={BADGES[check.state].variant}>{BADGES[check.state].label}</Badge>}
              control={fixControl(check)}
            />
          ))}
        </SettingsList>
        <p className="gh-ui-webhooks__checked">
          {checking
            ? 'Checking prerequisites…'
            : checkedAt
              ? `${summarize(status)} Last checked at ${checkedAt.toLocaleTimeString()}.`
              : summarize(status)}
        </p>
      </SettingsSection>
      <SettingsSection title="PR webhooks">
        <SettingsList>
          <SettingsRow
            layout="auto"
            label="Webhooks"
            description={stateLine}
            meta={<Badge variant={badge.variant}>{badge.label}</Badge>}
            control={
              status.enabled ? (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy !== null}
                  aria-busy={busy === 'enable'}
                  onClick={() => void setEnabled(false)}
                >
                  {busy === 'enable' ? 'Disabling…' : 'Disable'}
                </Button>
              ) : (
                <Button
                  size="sm"
                  variant="primary"
                  disabled={busy !== null || !status.ready}
                  aria-busy={busy === 'enable'}
                  onClick={() => void setEnabled(true)}
                >
                  {busy === 'enable' ? 'Enabling…' : 'Enable webhooks'}
                </Button>
              )
            }
            action={
              status.restart_required ? (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy !== null}
                  aria-busy={busy === 'restart'}
                  onClick={() => void restartConfirmed()}
                >
                  {busy === 'restart' ? 'Restarting…' : 'Restart github'}
                </Button>
              ) : undefined
            }
          />
        </SettingsList>
      </SettingsSection>
      {actionError ? (
        <div className="gh-ui-webhooks__failure" role="alert">
          <p className="gh-ui-webhooks__error">{actionError}</p>
          <Button size="sm" variant="ghost" onClick={() => setActionError(null)}>
            Dismiss
          </Button>
        </div>
      ) : null}
      {error ? (
        <p className="gh-ui-webhooks__error" role="status">
          Could not re-check: {error}
        </p>
      ) : null}
    </PageMain>
  )
}
