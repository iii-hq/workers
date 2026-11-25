import { useEffect, useId, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { Dialog } from '@/components/shadcn/dialog'
import { Input } from '@/components/shadcn/input'
import {
  FormDialogContent,
  FormDialogDescription,
  FormDialogTitle,
} from '@/components/ui/FormDialog'
import {
  cancelLogin,
  parseAccountsError,
  refreshClaudeAccounts,
  startLogin,
  submitLoginCode,
} from '@/lib/backend/claude-accounts'
import * as W from '@/lib/claude-sign-in-words'
import { copyTextToClipboard } from '@/lib/clipboard'
import { AccountDot } from './AccountCard'

export interface SignInTarget {
  id: string
  name: string
  color: string
  email: string | null
  /** Just added: "new account". */
  isNew?: boolean
  /** Signed in before: "sign in again". */
  again?: boolean
}

type Phase =
  | { kind: 'starting' }
  | {
      kind: 'waiting'
      loginId: string
      url: string
      expiresAtMs: number
      email: string | null
      hint: string | null
    }
  | {
      kind: 'checking'
      loginId: string
      url: string
      expiresAtMs: number
      email: string | null
    }
  | { kind: 'done'; email: string | null; asked: string | null }
  | { kind: 'stopped'; title: string; body: string }

/**
 * A Claude Code account's sign-in in the ADE (claude-login-ui, §4.3.1):
 * `tmux::accounts::login_start` with no browser on the engine, the link to
 * open here, the code pasted back, `login_code`. Done offers to make it the
 * global account when it isn't. Closing a sign-in under way cancels it.
 */
export function ClaudeSignInDialog({
  target,
  onOpenChange,
  globalName,
  globalWeek,
  isGlobal,
  onMakeGlobal,
}: {
  /** The account to sign in; null closes the dialog. */
  target: SignInTarget | null
  onOpenChange: (open: boolean) => void
  globalName: string | null
  globalWeek: number | null
  isGlobal: boolean
  onMakeGlobal?: (account: string) => void
}) {
  const open = target !== null
  const [phase, setPhase] = useState<Phase>({ kind: 'starting' })
  const [code, setCode] = useState('')
  const [copied, setCopied] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const loginRef = useRef<string | null>(null)
  const runRef = useRef(0)
  const ids = useId()
  const codeRef = useRef<HTMLInputElement>(null)

  const begin = async (account: SignInTarget) => {
    const run = ++runRef.current
    setPhase({ kind: 'starting' })
    setCode('')
    try {
      const started = await startLogin(account.id)
      if (run !== runRef.current) {
        void cancelLogin(started.loginId).catch(() => undefined)
        return
      }
      loginRef.current = started.loginId
      setPhase({
        kind: 'waiting',
        loginId: started.loginId,
        url: started.url,
        expiresAtMs: started.expiresAtMs,
        email: started.email ?? account.email,
        hint: null,
      })
    } catch (err) {
      if (run !== runRef.current) return
      const e = parseAccountsError(err)
      setPhase({
        kind: 'stopped',
        title: W.START_FAILED_TITLE,
        body: e.message || W.NO_REASON,
      })
    }
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: one sign-in per opening
  useEffect(() => {
    if (!target) return
    void begin(target)
    return () => {
      runRef.current++
      const loginId = loginRef.current
      loginRef.current = null
      if (loginId) void cancelLogin(loginId).catch(() => undefined)
    }
  }, [target?.id])

  // The countdown.
  useEffect(() => {
    if (phase.kind !== 'waiting') return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [phase.kind])

  if (!target) return null
  const name = target.name

  const submit = async () => {
    if (phase.kind !== 'waiting') return
    const cleaned = code.replace(/\s+/g, '')
    if (!cleaned) return
    const run = runRef.current
    const { loginId, url, expiresAtMs, email } = phase
    setPhase({ kind: 'checking', loginId, url, expiresAtMs, email })
    try {
      const account = await submitLoginCode(loginId, cleaned)
      if (run !== runRef.current) return
      loginRef.current = null
      setPhase({ kind: 'done', email: account?.email ?? null, asked: email })
      void refreshClaudeAccounts(true)
    } catch (err) {
      if (run !== runRef.current) return
      const e = parseAccountsError(err)
      switch (e.code) {
        case 'invalid_code':
          setPhase({
            kind: 'waiting',
            loginId,
            url,
            expiresAtMs,
            email,
            hint: e.message,
          })
          break
        case 'rejected':
          setPhase({
            kind: 'waiting',
            loginId,
            url,
            expiresAtMs,
            email,
            hint: W.REJECTED,
          })
          break
        case 'expired':
          loginRef.current = null
          setPhase({
            kind: 'stopped',
            title: W.EXPIRED_TITLE,
            body: W.expiredBody(
              Math.max(0, expiresAtMs - Date.now()) || W.DEFAULT_LINK_MS,
              email,
            ),
          })
          break
        case 'gone':
          loginRef.current = null
          setPhase({
            kind: 'stopped',
            title: W.CANCELLED_TITLE,
            body: W.cancelledBody(name, e.message),
          })
          break
        default:
          loginRef.current = null
          setPhase({
            kind: 'stopped',
            title: W.FAILED_TITLE,
            body: e.message || W.NO_REASON,
          })
      }
    }
  }

  const close = () => onOpenChange(false)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <FormDialogContent
        data-claude-sign-in={target.id}
        className="w-[min(30rem,calc(100vw-2rem))] [--dialog-max-width:30rem]"
        initialFocus={codeRef}
      >
        <div className="flex max-h-[min(40rem,calc(100dvh-2rem))] flex-col gap-4 overflow-y-auto p-5">
          <div className="flex flex-col gap-1 pr-8">
            <FormDialogTitle>
              {phase.kind === 'done'
                ? W.signedIn(name)
                : phase.kind === 'stopped'
                  ? phase.title
                  : W.TITLE}
            </FormDialogTitle>
            <FormDialogDescription>
              <span className="flex items-center gap-1.5">
                <AccountDot color={target.color} />
                {W.subtitle(name, Boolean(target.isNew), Boolean(target.again))}
              </span>
            </FormDialogDescription>
          </div>

          {phase.kind === 'starting' ? (
            <p role="status" className="text-sm text-ink-faint">
              {W.GETTING_LINK}
            </p>
          ) : null}

          {phase.kind === 'waiting' || phase.kind === 'checking' ? (
            <>
              <div className="flex flex-col gap-2">
                <span className="text-body font-medium text-ink">
                  {`1. ${W.STEP_OPEN}`}
                </span>
                <p className="text-sm text-ink-faint">
                  {W.openBody(name, phase.email)}
                </p>
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    size="sm"
                    nativeButton={false}
                    render={
                      <a
                        href={phase.url}
                        target="_blank"
                        rel="noopener noreferrer"
                      />
                    }
                  >
                    {W.OPEN}
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => {
                      void copyTextToClipboard(phase.url).then(() =>
                        setCopied(true),
                      )
                    }}
                  >
                    {copied ? W.LINK_COPIED : W.COPY_LINK}
                  </Button>
                  <span className="text-xs text-ink-faint tabular-nums">
                    {W.linkWorks(W.DEFAULT_LINK_MS)}
                    {phase.expiresAtMs > 0
                      ? ` · ${W.left(phase.expiresAtMs, now)} left`
                      : ''}
                  </span>
                </div>
              </div>
              <form
                noValidate
                className="flex flex-col gap-2"
                onSubmit={(e) => {
                  e.preventDefault()
                  void submit()
                }}
              >
                <label
                  htmlFor={`${ids}-code`}
                  className="text-body font-medium text-ink"
                >
                  {`2. ${W.STEP_PASTE}`}
                </label>
                <Input
                  ref={codeRef}
                  id={`${ids}-code`}
                  name="claude-sign-in-code"
                  value={code}
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={W.PLACEHOLDER}
                  disabled={phase.kind === 'checking'}
                  aria-invalid={
                    phase.kind === 'waiting' && phase.hint ? true : undefined
                  }
                  onChange={(e) => setCode(e.currentTarget.value)}
                />
                {phase.kind === 'waiting' && phase.hint ? (
                  <p role="alert" className="text-xs text-alert">
                    {phase.hint}
                  </p>
                ) : null}
                {phase.kind === 'checking' ? (
                  <p role="status" className="text-xs text-ink-faint">
                    {W.CHECKING}
                  </p>
                ) : null}
                <div className="flex items-center justify-end gap-2 pt-1">
                  <Button type="button" variant="secondary" onClick={close}>
                    {W.CANCEL}
                  </Button>
                  <Button
                    type="submit"
                    disabled={
                      phase.kind === 'checking' || !code.replace(/\s+/g, '')
                    }
                  >
                    {phase.kind === 'checking' ? W.SIGNING_IN : W.SIGN_IN}
                  </Button>
                </div>
              </form>
            </>
          ) : null}

          {phase.kind === 'done' ? (
            <>
              {phase.email ? (
                <p className="text-sm text-ink">{phase.email}</p>
              ) : null}
              {W.otherPerson(phase.asked, phase.email) ? (
                <p className="text-sm text-warn">
                  {W.otherPerson(phase.asked, phase.email)}
                </p>
              ) : null}
              <p className="text-sm text-ink-faint">
                {isGlobal || !globalName || !onMakeGlobal
                  ? isGlobal
                    ? W.isGlobal(name)
                    : ''
                  : W.offer(name, globalName, globalWeek)}
              </p>
              <div className="flex items-center justify-end gap-2">
                {isGlobal || !onMakeGlobal ? (
                  <Button onClick={close}>{W.DONE}</Button>
                ) : (
                  <>
                    <Button variant="secondary" onClick={close}>
                      {W.NOT_NOW}
                    </Button>
                    <Button
                      onClick={() => {
                        close()
                        onMakeGlobal(target.id)
                      }}
                    >
                      {W.makeGlobal(name)}
                    </Button>
                  </>
                )}
              </div>
            </>
          ) : null}

          {phase.kind === 'stopped' ? (
            <>
              <p className="text-sm text-ink-faint">{phase.body}</p>
              <div className="flex items-center justify-end gap-2">
                <Button variant="secondary" onClick={close}>
                  {W.CLOSE}
                </Button>
                <Button onClick={() => void begin(target)}>{W.NEW_LINK}</Button>
              </div>
            </>
          ) : null}
        </div>
      </FormDialogContent>
    </Dialog>
  )
}
