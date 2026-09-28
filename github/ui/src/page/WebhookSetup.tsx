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
  PageMain,
  SettingsList,
  SettingsRow,
  SettingsSection,
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

export function WebhookSetup({ host }: { host: Host }) {
  const [status, setStatus] = useState<SetupStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const { confirm, dialog } = useConfirm()
  const latest = useRef(0)

  const refresh = useCallback(async () => {
    const request = ++latest.current
    try {
      const next = await host.iii.trigger<SetupStatus>(STATUS_FN, {}, { timeoutMs: 20_000 })
      if (request === latest.current) {
        setStatus(next)
        setError(null)
      }
    } catch (err) {
      if (request === latest.current) setError(errorMessage(err))
    }
  }, [host])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const run = useCallback(
    async (key: string, action: () => Promise<unknown>) => {
      setBusy(key)
      try {
        await action()
      } catch (err) {
        setError(errorMessage(err))
      } finally {
        setBusy(null)
        await refresh()
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
    if (ok) {
      await run(`install:${worker}`, () =>
        host.iii.trigger('compose::add', { workers: [worker] }, { timeoutMs: 600_000 }),
      )
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

  const restartGithub = () =>
    host.iii.trigger('compose::restart', { container: GITHUB_CONTAINER }, { timeoutMs: 180_000 })

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
            action={{ label: 'Try again', onClick: () => void refresh() }}
          />
        ) : (
          <p className="gh-ui-webhooks__loading" role="status">
            Checking webhook prerequisites…
          </p>
        )}
      </PageMain>
    )
  }

  const stateLine = status.active
    ? 'Active: watched pull requests receive GitHub deliveries.'
    : status.enabled
      ? 'Enabled in configuration; restart the github worker to apply it.'
      : status.ready
        ? 'Every prerequisite is ready.'
        : 'Complete the steps above to enable PR webhooks.'

  return (
    <PageMain className="gh-ui-main gh-ui-webhooks">
      {dialog}
      <SettingsSection
        title="PR webhook setup"
        description="github::pr::watch needs these on this machine. Nothing is installed without asking, and cloudflared is never downloaded for you."
        action={
          <Button size="sm" variant="ghost" disabled={busy !== null} onClick={() => void refresh()}>
            <RefreshCw aria-hidden /> Check again
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
      </SettingsSection>
      <SettingsSection title="PR webhooks">
        <SettingsList>
          <SettingsRow
            layout="auto"
            label="Webhooks"
            description={stateLine}
            meta={
              <Badge variant={status.active ? 'ok' : 'default'}>
                {status.active ? 'On' : status.enabled ? 'Restart pending' : 'Off'}
              </Badge>
            }
            control={
              status.enabled ? (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy !== null}
                  onClick={() => void setEnabled(false)}
                >
                  Disable
                </Button>
              ) : (
                <Button
                  size="sm"
                  variant="primary"
                  disabled={busy !== null || !status.ready}
                  onClick={() => void setEnabled(true)}
                >
                  {busy === 'enable' ? 'Enabling…' : 'Enable webhooks'}
                </Button>
              )
            }
            action={
              status.restart_required && status.enabled ? (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy !== null}
                  onClick={() => void run('restart', restartGithub)}
                >
                  Restart github
                </Button>
              ) : undefined
            }
          />
        </SettingsList>
      </SettingsSection>
      {error ? (
        <p className="gh-ui-webhooks__error" role="alert">
          {error}
        </p>
      ) : null}
    </PageMain>
  )
}
