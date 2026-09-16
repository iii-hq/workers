import { Button, type Host, type ProviderConfigFormProps } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { useCodexAuth, type LoginStartResponse } from './src/use-codex-auth'

/** Replaced by the worker at serve time with its `CODEX_COMPAT_VERSION`. */
const COMPAT_VERSION = '__CODEX_COMPAT_VERSION__'

function verificationLink(uri: string): string | null {
  try {
    const url = new URL(uri)
    return url.protocol === 'https:' && !url.username && !url.password ? url.href : null
  } catch {
    return null
  }
}

function LoginInstructions({
  login,
  canceling,
  onCancel,
}: {
  login: LoginStartResponse
  canceling: boolean
  onCancel(): void
}) {
  const [now, setNow] = useState(Date.now)
  const [copying, setCopying] = useState(false)
  const [clipboard, setClipboard] = useState<string | null>(null)
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  useEffect(() => {
    if (!clipboard) return
    const timer = setTimeout(() => setClipboard(null), 3000)
    return () => clearTimeout(timer)
  }, [clipboard])
  const remaining = Math.max(0, Math.ceil(login.expires_at - now / 1000))
  const link = verificationLink(login.verification_uri)
  async function copyCode() {
    setCopying(true)
    setClipboard(null)
    try {
      await navigator.clipboard.writeText(login.user_code)
      setClipboard('Code copied.')
    } catch {
      setClipboard('Could not copy. Select and copy the code manually.')
    } finally {
      setCopying(false)
    }
  }
  return (
    <div className="codex-provider-login">
      <ol>
        <li>
          {link ? (
            <a href={link} target="_blank" rel="noopener noreferrer">
              Open ChatGPT sign-in (new tab)
            </a>
          ) : (
            'The sign-in link is unavailable. Cancel and start a new attempt.'
          )}
        </li>
        <li>Enter this code and approve access:</li>
      </ol>
      <div className="codex-provider-action">
        <code className="codex-provider-code">{login.user_code}</code>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={copying || canceling || remaining === 0}
          onClick={() => void copyCode()}
        >
          {copying ? 'Copying…' : 'Copy code'}
        </Button>
      </div>
      <p role="status">{clipboard ?? 'Waiting for approval in your browser…'}</p>
      <div className="codex-provider-action">
        <span role="timer" aria-live="off">
          Code expires in {Math.floor(remaining / 60)}:{String(remaining % 60).padStart(2, '0')}
        </span>
        <Button type="button" variant="ghost" size="sm" disabled={canceling} onClick={onCancel}>
          {canceling ? 'Canceling…' : 'Cancel sign-in'}
        </Button>
      </div>
    </div>
  )
}

function CodexProviderForm({ host }: ProviderConfigFormProps & { host: Host }) {
  const state = useCodexAuth(host.iii)
  const authenticated = state.auth?.status === 'authenticated'
  const expired = state.auth?.status === 'expired'
  const disabled = Boolean(state.busy || state.confirming)
  const status = state.auth
    ? authenticated
      ? 'Signed in'
      : expired
        ? 'Session expired'
        : 'Signed out'
    : state.checking
      ? 'Checking account…'
      : 'Account status unavailable'
  return (
    <div className="codex-provider-form">
      <section className="codex-provider-card">
        <div className="codex-provider-status" role="status">
          <span className="codex-provider-dot" data-active={authenticated} aria-hidden="true" />
          <span>{status}</span>
        </div>
        <h3>Connect your ChatGPT account</h3>
        <p>
          This provider uses your ChatGPT subscription. Do not enter an API key here; API keys belong to the OpenAI
          provider.
        </p>
        <p>One account per namespace. Managed sign-ins refresh automatically.</p>
        {state.auth?.source && (
          <p className="codex-provider-account">
            {state.auth.account_id ? (
              <>
                Account: <strong>{state.auth.account_id}</strong>.{' '}
              </>
            ) : null}
            {state.auth.source === 'local'
              ? 'Using the local Codex login.'
              : state.auth.source === 'vault'
                ? 'Using the saved vault login.'
                : 'Managed by this provider.'}
          </p>
        )}
        {state.login && (
          <>
            {authenticated && <p>Your current account stays signed in until the new sign-in succeeds.</p>}
            <LoginInstructions
              key={state.login.login_id}
              login={state.login}
              canceling={state.busy === 'cancel'}
              onCancel={() => void state.cancel()}
            />
          </>
        )}
        {state.statusError && <p role="alert">{state.statusError}</p>}
        {state.actionError && <p role="alert">{state.actionError}</p>}
        {state.pollError && <p role="status">{state.pollError}</p>}
        {state.notice && <p role="status">{state.notice}</p>}
        <p>Enable device code login in ChatGPT security settings or ask your workspace administrator.</p>
        <div className="codex-provider-action">
          {!state.login && state.auth && (
            <Button type="button" variant="primary" size="sm" disabled={disabled} onClick={() => void state.start()}>
              {state.busy === 'start'
                ? 'Starting sign-in…'
                : state.confirming
                  ? 'Checking sign-in…'
                  : state.retry
                    ? 'New attempt'
                    : authenticated
                      ? 'Switch account'
                      : expired
                        ? 'Reconnect'
                        : 'Sign in with ChatGPT'}
            </Button>
          )}
          {state.statusError && (
            <Button
              type="button"
              size="sm"
              disabled={Boolean(state.busy) || state.checking || Boolean(state.login)}
              onClick={() => void state.checkStatus()}
            >
              {state.checking ? 'Checking…' : 'Retry status'}
            </Button>
          )}
          {(authenticated || expired) && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={disabled || Boolean(state.login)}
              onClick={() => void state.logout()}
            >
              {state.busy === 'logout' ? 'Logging out…' : 'Log out'}
            </Button>
          )}
          {authenticated && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={disabled || state.refreshing || Boolean(state.login)}
              onClick={() => void state.refreshModels()}
            >
              {state.refreshing ? 'Refreshing models…' : 'Refresh models'}
            </Button>
          )}
        </div>
        {state.catalogMessage && <p role="status">{state.catalogMessage}</p>}
        <div className="codex-provider-help">
          <p>
            Already use Codex locally? Run <code>codex login</code> on the machine running this provider. Your local or
            vault login works until you connect a managed account or explicitly log out.
          </p>
          <p>
            Mirrors Codex CLI <code>{COMPAT_VERSION}</code>. If newer CLI models are missing here, this provider may
            need a version update.
          </p>
        </div>
      </section>
    </div>
  )
}

export default function setup(host: Host) {
  host.providerConfigForms?.register('openai-codex', (props) => <CodexProviderForm host={host} {...props} />)
}
