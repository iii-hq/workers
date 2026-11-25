import { ChevronDown, Clock, X } from 'lucide-react'
import { useState } from 'react'
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
  parseAccountsError,
} from '@/lib/backend/claude-accounts'
import {
  accountUse,
  continueIn,
  HIDE_LIMIT,
  LIMIT_HINT,
  limitResetLine,
  limitTitle,
  makeGlobal,
  mostRoom,
  OTHER,
  SCREEN_TITLE,
  sideChatLimitHint,
  type TimeStyle,
  UNKNOWN_ACCOUNT_LINE,
  unknownAccountTitle,
  usable,
} from '@/lib/claude-accounts-words'
import type { LimitCardModel } from '@/lib/limit-card'
import { AccountDot } from './AccountCard'

export interface LimitCardProps {
  model: LimitCardModel
  data: ClaudeAccounts
  time: TimeStyle
  /** A side chat: the title, the reset line and the X only (its account is locked, D2). */
  sideChat?: boolean
  /** The conversation follows the global account, and the limited one is it. */
  followsGlobal?: boolean
  /** Pin the conversation to `account`, then re-run the failed turn. */
  onContinue: (account: string) => Promise<void>
  /** Make `account` the global one, then re-run the failed turn. */
  onMakeGlobal: (account: string) => Promise<void>
  /** "Claude Code accounts…" in Other…. */
  onOpenAccounts: () => void
  onDismiss: () => void
}

/**
 * The limit card (§4.3.4, D135), in place of "Turn ended · Continue" while
 * it shows: "Personal reached its 5-hour limit", when it resets, "Continue
 * in Work" (the account with the most room), "Other…" (the accounts with
 * their use), and "Make Work the global" when the conversation follows the
 * global. A side chat keeps its account: only the title, the reset line and
 * the X.
 */
export function LimitCard({
  model,
  data,
  time,
  sideChat = false,
  followsGlobal = false,
  onContinue,
  onMakeGlobal,
  onOpenAccounts,
  onDismiss,
}: LimitCardProps) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const limited = data.account(model.account)
  const name = limited?.name ?? model.account
  const best = mostRoom(data.claude, model.account)
  const others = (data.claude?.accounts ?? []).filter(
    (a) => a.id !== model.account,
  )
  const title =
    model.kind === 'unknown'
      ? unknownAccountTitle(model.account)
      : limitTitle(name, model.window)
  const line =
    model.kind === 'unknown'
      ? UNKNOWN_ACCOUNT_LINE
      : limitResetLine(model.resetsAtMs, time)

  const run = (action: () => Promise<void>) => {
    if (busy) return
    setBusy(true)
    setError(null)
    action()
      .catch((err: unknown) => setError(parseAccountsError(err).message))
      .finally(() => setBusy(false))
  }

  return (
    <section
      data-limit-card={model.kind}
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

      {sideChat ? null : (
        <div className="flex flex-wrap items-center gap-2">
          {best ? (
            <Button
              data-limit-continue={best.id}
              disabled={busy}
              onClick={() => run(() => onContinue(best.id))}
            >
              <AccountDot color={best.color} />
              {continueIn(best.name)}
            </Button>
          ) : null}
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
                  onClick={() => run(() => onContinue(a.id))}
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
          {followsGlobal && best && model.kind !== 'unknown' ? (
            <Button
              variant="secondary"
              data-limit-make-global={best.id}
              disabled={busy}
              onClick={() => run(() => onMakeGlobal(best.id))}
            >
              {makeGlobal(best.name)}
            </Button>
          ) : null}
        </div>
      )}

      {error ? (
        <p role="alert" className="text-xs text-alert">
          {error}
        </p>
      ) : null}
      <p className="text-xs text-ink-faint">
        {sideChat ? sideChatLimitHint(name) : LIMIT_HINT}
      </p>
    </section>
  )
}
