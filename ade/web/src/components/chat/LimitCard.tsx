import { ChevronDown, Clock, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  DropdownMenu,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { ChatMenuContent, ChatMenuItem } from '@/components/ui/ChatMenu'
import { QuietIconButton } from '@/components/ui/QuietIconButton'
import {
  type ClaudeAccounts,
  type GlobalResult,
  parseAccountsError,
} from '@/lib/backend/claude-accounts'
import {
  accountUse,
  continueIn,
  HIDE_LIMIT,
  LIMIT_HAS_RESET,
  LIMIT_HINT,
  type LockedAccount,
  limitResetLine,
  limitTitle,
  lockedAccountRemoved,
  lockedLimitHint,
  makeGlobal,
  mostRoom,
  OTHER,
  SCREEN_TITLE,
  type TimeStyle,
  TRY_AGAIN,
  UNKNOWN_ACCOUNT_LINE,
  unknownAccountTitle,
  usable,
  waitingBanner,
} from '@/lib/claude-accounts-words'
import type { LimitCardModel } from '@/lib/limit-card'
import { AccountDot } from './AccountCard'

export interface LimitCardProps {
  model: LimitCardModel
  data: ClaudeAccounts
  time: TimeStyle
  /**
   * The conversation can't choose its account (a side chat, D2, or a
   * sub-agent): the title, the line, its hint and the X only.
   */
  locked?: LockedAccount
  /** The conversation follows the global account, and the limited one is it. */
  followsGlobal?: boolean
  /** Pin the conversation to `account` (`claude_account`, a patch of that key alone). */
  onPin: (account: string) => Promise<void>
  /** Make `account` the global one; rejects with the refusal's words. */
  onSetGlobal: (account: string) => Promise<GlobalResult>
  /** Re-run the failed turn (`harness::retry`) on the account it resolves to now. */
  onRetry: () => Promise<void>
  /** "Claude Code accounts…" in Other…. */
  onOpenAccounts: () => void
  onDismiss: () => void
}

/** What a button did: re-ran the turn (the card waits for it), or not. */
type Outcome = 'retried' | 'done'

/**
 * The limit card (§4.3.4, D135), above the composer while it shows:
 * "Personal reached its 5-hour limit", when it resets, "Continue
 * in Work" (the account with the most room, when one has room), "Other…"
 * (the accounts with their use), and "Make Work the global" when the
 * conversation follows the global. Once the window has reset: "The limit
 * has reset" and "Try again" on the same account. A side chat or a
 * sub-agent keeps its account: only the title, the line, its hint and the X.
 *
 * A turn re-run keeps the buttons off until the card changes (the turn
 * starts and puts it away). A global switch tmux has to wait for shows its
 * waiting words; the turn runs again once the switch is done.
 */
export function LimitCard({
  model,
  data,
  time,
  locked = null,
  followsGlobal = false,
  onPin,
  onSetGlobal,
  onRetry,
  onOpenAccounts,
  onDismiss,
}: LimitCardProps) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  /** A global switch tmux answered `waiting` for, and its words. */
  const [waiting, setWaiting] = useState<{
    account: string
    text: string
  } | null>(null)
  const sawPending = useRef(false)
  const limited = data.account(model.account)
  const name = limited?.name ?? model.account
  const best = mostRoom(data.claude, model.account)
  const others = (data.claude?.accounts ?? []).filter(
    (a) => a.id !== model.account,
  )
  const failedTurn = model.kind !== 'reading'
  const global = data.claude?.global ?? null
  const pending = data.claude?.pendingGlobal ?? null
  const title =
    model.kind === 'unknown'
      ? unknownAccountTitle(model.account)
      : limitTitle(name, model.window)
  const line =
    model.kind === 'unknown'
      ? locked
        ? lockedAccountRemoved(locked)
        : UNKNOWN_ACCOUNT_LINE
      : model.reset
        ? LIMIT_HAS_RESET
        : limitResetLine(model.resetsAtMs, time)
  // A removed account's line already says why nothing can be chosen here.
  const hint = !locked
    ? LIMIT_HINT
    : model.kind === 'unknown'
      ? null
      : lockedLimitHint(locked, name)
  const waitingText = pending
    ? waitingBanner(data.nameOf(pending.account), pending.waitingFor.length)
    : (waiting?.text ?? null)

  // The card's state changed (another failure, another reading, the reset):
  // the buttons are on again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the card's identity, not its handlers
  useEffect(() => {
    setBusy(false)
  }, [model.key, model.kind, model.reset])

  const run = (action: () => Promise<Outcome>) => {
    if (busy) return
    setBusy(true)
    setError(null)
    action().then(
      // A re-run turn takes the card away; until then nothing more to press.
      (outcome) => {
        if (outcome !== 'retried') setBusy(false)
      },
      (err: unknown) => {
        setError(parseAccountsError(err).message)
        setBusy(false)
      },
    )
  }
  const retryNow = async (): Promise<Outcome> => {
    await onRetry()
    return 'retried'
  }
  const retryRef = useRef<() => void>(() => {})
  retryRef.current = () => run(retryNow)

  // A waiting switch: done when the global is the account asked for (then
  // the failed turn runs again), off when tmux dropped it without that.
  useEffect(() => {
    if (!waiting) return
    if (pending?.account === waiting.account) {
      sawPending.current = true
      return
    }
    if (global === waiting.account) {
      setWaiting(null)
      if (failedTurn) retryRef.current()
      return
    }
    if (sawPending.current) setWaiting(null)
  }, [waiting, pending, global, failedTurn])

  const continueTo = (account: string) =>
    run(async (): Promise<Outcome> => {
      setWaiting(null)
      await onPin(account)
      return failedTurn ? retryNow() : 'done'
    })

  const switchGlobal = (account: string) =>
    run(async (): Promise<Outcome> => {
      const result = await onSetGlobal(account)
      if (result.state === 'waiting') {
        sawPending.current = false
        setWaiting({
          account,
          text: waitingBanner(data.nameOf(account), result.waitingFor.length),
        })
        return 'done'
      }
      return failedTurn ? retryNow() : 'done'
    })

  const primary = model.reset ? (
    <Button data-limit-retry="" disabled={busy} onClick={() => run(retryNow)}>
      {TRY_AGAIN}
    </Button>
  ) : best ? (
    <Button
      data-limit-continue={best.id}
      disabled={busy}
      onClick={() => continueTo(best.id)}
    >
      <AccountDot color={best.color} />
      {continueIn(best.name)}
    </Button>
  ) : null

  return (
    <section
      data-limit-card={model.kind}
      data-limit-reset={model.reset || undefined}
      aria-labelledby={`limit-card-${model.key}`}
      className="mb-2 flex flex-col gap-3 rounded-xl border border-border bg-popover px-4 py-3.5 shadow-sm"
    >
      <div className="flex items-start gap-3">
        <span
          aria-hidden
          className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-alert-muted text-alert"
        >
          <Clock className="size-4" />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <h3
            id={`limit-card-${model.key}`}
            className="text-sm font-semibold text-ink"
          >
            {title}
          </h3>
          {line ? <p className="text-body text-ink-faint">{line}</p> : null}
        </div>
        <QuietIconButton
          aria-label={HIDE_LIMIT}
          title={HIDE_LIMIT}
          data-limit-dismiss=""
          onClick={onDismiss}
        >
          <X aria-hidden className="size-4" />
        </QuietIconButton>
      </div>

      {locked ? null : (
        <div className="flex flex-wrap items-center gap-2">
          {primary}
          <DropdownMenu>
            <DropdownMenuTrigger
              disabled={busy}
              data-limit-other=""
              render={<Button variant="secondary" />}
            >
              {OTHER}
              <ChevronDown aria-hidden />
            </DropdownMenuTrigger>
            <ChatMenuContent align="start" sideOffset={6} className="min-w-64">
              {others.map((a) => (
                <ChatMenuItem
                  key={a.id}
                  data-limit-account={a.id}
                  disabled={!usable(a)}
                  onClick={() => continueTo(a.id)}
                >
                  <AccountDot color={a.color} />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-ink">{a.name}</span>
                    <span className="truncate text-xs text-ink-faint">
                      {accountUse(a)}
                    </span>
                  </span>
                </ChatMenuItem>
              ))}
              {others.length > 0 ? (
                <DropdownMenuSeparator className="my-1.5" />
              ) : null}
              <ChatMenuItem onClick={onOpenAccounts}>
                {`${SCREEN_TITLE}…`}
              </ChatMenuItem>
            </ChatMenuContent>
          </DropdownMenu>
          {followsGlobal && best && !model.reset && model.kind !== 'unknown' ? (
            <Button
              variant="secondary"
              data-limit-make-global={best.id}
              disabled={busy || pending?.account === best.id}
              onClick={() => switchGlobal(best.id)}
            >
              {makeGlobal(best.name)}
            </Button>
          ) : null}
        </div>
      )}

      {waitingText && !locked ? (
        <p role="status" data-limit-waiting="" className="text-xs text-ink">
          {waitingText}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-xs text-alert">
          {error}
        </p>
      ) : null}
      {hint ? <p className="text-xs text-ink-faint">{hint}</p> : null}
    </section>
  )
}
