import { Button, Input, StatusDot, StatusPanel, useConfirm } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Copy, ExternalLink } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import type { Api } from './api'
import { countdown, signInControls } from './presentation'
import { CopyIconButton } from './SchemaTable'
import type { Server } from './types'
import { useLogin } from './use-login'

/** Interactive sign-in for the Connection view: Connect, the waiting view with its paste fallback, a failed
 * attempt, and Sign out. A raise of `startSignal` (the row's Sign in) starts an attempt. `quiet` while the save
 * bar shows: Save is then the view's default button. */
export function SignIn({
  api,
  server,
  startSignal,
  quiet = false,
}: {
  api: Api
  server: Server
  startSignal: number
  quiet?: boolean
}) {
  const login = useLogin(api, server)
  const { phase, link, start } = login
  const { confirm, dialog } = useConfirm()
  // Holds an OAuth code: never kept once the field is gone.
  const [pasted, setPasted] = useState('')
  const [signOutError, setSignOutError] = useState<string | null>(null)
  const copyLink = useCopyFlash(link ?? '')
  const seen = useRef(0)
  const pasteId = useId()

  useEffect(() => {
    if (startSignal > 0 && startSignal !== seen.current) start()
    seen.current = startSignal
  }, [startSignal, start])
  useEffect(() => {
    if (!login.paste) setPasted('')
  }, [login.paste])
  // As the hook's own error: a server that moved on leaves no old failure behind.
  useEffect(() => {
    setSignOutError(null)
  }, [server.state])

  const busy = phase === 'starting' || phase === 'finishing'
  const { show, connect, signOut } = signInControls(server, phase)
  if (!show) return dialog

  const primary = quiet ? 'secondary' : 'primary'
  const finish = () => {
    if (busy || !pasted.trim()) return
    setPasted('')
    login.finish(pasted)
  }
  const onSignOut = async () => {
    const ok = await confirm({
      title: `Sign out of ${server.name}?`,
      description: 'Its functions stay registered and answer with a sign-in error until someone signs in again.',
      confirmLabel: 'Sign out',
    })
    if (!ok) return
    setSignOutError(null)
    try {
      await api.signOut(server.name)
    } catch (err) {
      setSignOutError(errorMessage(err))
    }
  }

  return (
    <section className="mcp-ui-signin" aria-label="Sign-in" data-phase={phase}>
      {phase === 'failed' ? (
        <StatusPanel
          variant="alert"
          role="alert"
          headline={login.error ?? 'The sign-in attempt expired.'}
          action={
            <Button size="sm" variant={primary} onClick={start}>
              Try again
            </Button>
          }
        />
      ) : null}
      {connect ? (
        <div className="mcp-ui-run">
          <Button size="sm" variant={primary} disabled={busy} onClick={start}>
            Connect
          </Button>
        </div>
      ) : null}
      {signOut ? (
        <div className="mcp-ui-run">
          <Button size="sm" variant="ghost" onClick={() => void onSignOut()}>
            Sign out
          </Button>
        </div>
      ) : null}
      {login.timer && login.expiresAt !== null ? (
        <>
          <p className="mcp-ui-result-status">
            <StatusDot tone="accent" pulse />
            <span>
              Waiting for sign-in ·{' '}
              <span role="timer" aria-live="off">
                {countdown(login.expiresAt, Date.now())}
              </span>
            </span>
          </p>
          <div className="mcp-ui-run">
            {phase === 'waiting' && link ? (
              <Button asChild size="sm" variant={primary}>
                <a href={link} target="_blank" rel="noopener noreferrer">
                  <ExternalLink size={16} aria-hidden />
                  Open sign-in page
                </a>
              </Button>
            ) : null}
            {phase === 'waiting' && !link ? (
              <span className="mcp-ui-quiet-line">
                The sign-in link is unavailable. Cancel and start a new attempt.
              </span>
            ) : null}
            {login.startAgain ? (
              <Button size="sm" variant={primary} onClick={start}>
                Start again
              </Button>
            ) : null}
            <Button size="sm" variant="ghost" disabled={busy} onClick={login.cancel}>
              Cancel
            </Button>
          </div>
          <details className="mcp-ui-optional" open={phase === 'resumed' || undefined}>
            <summary>Browser on another machine?</summary>
            <div>
              {login.canCopyLink ? (
                <Button variant="secondary" size="sm" onClick={copyLink.copy}>
                  <Copy size={16} aria-hidden />
                  {copyLink.state === 'copied' ? 'Copied' : 'Copy sign-in link'}
                </Button>
              ) : null}
              {login.redirectUri ? (
                <p>
                  <span>Redirect URI</span>
                  <code>{login.redirectUri}</code>
                  <CopyIconButton value={login.redirectUri} label="Copy redirect URI" />
                </p>
              ) : null}
              <form
                onSubmit={(event) => {
                  event.preventDefault()
                  finish()
                }}
              >
                <label htmlFor={pasteId}>Paste the address the browser ended on</label>
                <div>
                  <Input
                    id={pasteId}
                    value={pasted}
                    disabled={busy}
                    spellCheck={false}
                    autoComplete="off"
                    onChange={setPasted}
                  />
                  <Button type="submit" size="sm" variant="secondary" disabled={busy || !pasted.trim()}>
                    Finish sign-in
                  </Button>
                </div>
              </form>
            </div>
          </details>
        </>
      ) : null}
      {busy ? (
        <p className="mcp-ui-quiet-line" role="status">
          {phase === 'starting' ? 'Starting sign-in…' : 'Finishing sign-in…'}
        </p>
      ) : null}
      {signOutError ? (
        <StatusPanel variant="alert" role="alert" headline="Could not sign out" detail={signOutError} />
      ) : null}
      {dialog}
    </section>
  )
}
