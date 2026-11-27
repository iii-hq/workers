import { type ReactNode, useEffect, useState } from 'react'
import { RadioGroup } from '@/components/shadcn/radio-group'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import {
  type ClaudeAccount,
  type ClaudeAccounts,
  cancelLogin,
  parseAccountsError,
  refreshClaudeAccounts,
  setGlobalAccount,
} from '@/lib/backend/claude-accounts'
import {
  globalRefusal,
  globalToast,
  removeGlobalFirst,
  SIGN_IN,
  STILL_SWITCHING,
  signInFirstBody,
  signInFirstTitle,
  type TimeStyle,
  waitingBanner,
  weekPercent,
} from '@/lib/claude-accounts-words'
import { cn } from '@/lib/utils'
import { AccountCard, type AccountCardProps } from './AccountCard'
import {
  AccountColorDialog,
  AddAccountDialog,
  RemoveAccountDialog,
  RenameAccountDialog,
} from './AccountDialogs'
import { ClaudeSignInDialog, type SignInTarget } from './ClaudeSignInDialog'

/** Now, again every 30 s: the "4 min ago" and the resets move on. */
export function useUsageNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs)
    return () => window.clearInterval(timer)
  }, [intervalMs])
  return now
}

export interface AccountStatus {
  tone: 'info' | 'alert'
  text: string
}

export interface AccountActions {
  switching: boolean
  status: AccountStatus | null
  clearStatus: () => void
  /** The radio, the sign-in's offer, the limit card: make `id` the global account. */
  makeGlobal: (id: string) => void
  openAdd: () => void
  openSignIn: (account: ClaudeAccount) => void
  /** What a card needs to act on its account. */
  cardProps: (
    account: ClaudeAccount,
  ) => Pick<
    AccountCardProps,
    | 'switching'
    | 'signingIn'
    | 'onSignIn'
    | 'onCancelSignIn'
    | 'onRename'
    | 'onColor'
    | 'onRemove'
  >
  /** The dialogs the actions open; render them once. */
  dialogs: ReactNode
}

function target(a: ClaudeAccount, isNew = false): SignInTarget {
  return {
    id: a.id,
    name: a.name,
    color: a.color,
    email: a.email,
    isNew,
    again: !isNew && (a.signedIn || a.tokenExpired),
  }
}

/**
 * The accounts' actions, as the tmux accounts list has them: the global
 * switch (`tmux::accounts::global`, its waiting and refused states), the
 * sign-in, Add account, Rename…, Color… and Remove….
 */
export function useAccountActions(data: ClaudeAccounts): AccountActions {
  const claude = data.claude
  const [switching, setSwitching] = useState(false)
  const [status, setStatus] = useState<AccountStatus | null>(null)
  const [signInFirst, setSignInFirst] = useState<ClaudeAccount | null>(null)
  const [signIn, setSignIn] = useState<SignInTarget | null>(null)
  const [adding, setAdding] = useState(false)
  const [renaming, setRenaming] = useState<ClaudeAccount | null>(null)
  const [coloring, setColoring] = useState<ClaudeAccount | null>(null)
  const [removing, setRemoving] = useState<ClaudeAccount | null>(null)
  // A switch's outcome reads like the apps' toast: it goes after a while;
  // a refusal stays until the next action.
  useEffect(() => {
    if (status?.tone !== 'info') return
    const timer = window.setTimeout(() => setStatus(null), 12_000)
    return () => window.clearTimeout(timer)
  }, [status])

  const makeGlobal = (id: string) => {
    if (!claude || id === claude.global) return
    if (switching) {
      setStatus({ tone: 'info', text: STILL_SWITCHING })
      return
    }
    const to = data.account(id)
    if (to && !to.signedIn) {
      setSignInFirst(to)
      return
    }
    const name = to?.name ?? id
    setSwitching(true)
    setStatus(null)
    setGlobalAccount(id)
      .then((r) => {
        if (r.state === 'waiting') {
          setStatus({
            tone: 'info',
            text: waitingBanner(name, r.waitingFor.length),
          })
        } else {
          setStatus({
            tone: 'info',
            text: globalToast(
              data.nameOf(r.global),
              r.following,
              r.restarted.length,
              r.afterTurn.length,
              claude.switchDelayS,
            ),
          })
        }
      })
      .catch((err: unknown) => {
        const e = parseAccountsError(err)
        if (e.code === 'not_signed_in' && to) {
          setSignInFirst(to)
          return
        }
        setStatus({ tone: 'alert', text: globalRefusal(e, name) })
      })
      .finally(() => {
        setSwitching(false)
        void refreshClaudeAccounts()
      })
  }

  const loginFor = (id: string) =>
    claude?.logins.find(
      (l) =>
        l.account === id &&
        (l.state === 'waiting_code' || l.state === 'checking'),
    ) ?? null

  const cardProps: AccountActions['cardProps'] = (a) => {
    const login = loginFor(a.id)
    return {
      switching,
      signingIn: login !== null && signIn?.id !== a.id,
      onSignIn: () => setSignIn(target(a)),
      onCancelSignIn: login
        ? () => {
            void cancelLogin(login.loginId)
              .catch(() => undefined)
              .finally(() => void refreshClaudeAccounts())
          }
        : undefined,
      onRename: () => setRenaming(a),
      onColor: () => setColoring(a),
      onRemove: () => {
        if (claude && a.id === claude.global) {
          setStatus({ tone: 'alert', text: removeGlobalFirst(a.name) })
          return
        }
        setRemoving(a)
      },
    }
  }

  const globalAccount = data.account(claude?.global)
  const signInGlobal = signIn !== null && claude?.global === signIn.id
  const dialogs = (
    <>
      <ConfirmDialog
        open={signInFirst !== null}
        onOpenChange={(open) => !open && setSignInFirst(null)}
        title={signInFirst ? signInFirstTitle(signInFirst.name) : ''}
        description={signInFirst ? signInFirstBody(signInFirst.name) : ''}
        confirmLabel={SIGN_IN}
        onConfirm={() => {
          if (signInFirst) setSignIn(target(signInFirst))
          setSignInFirst(null)
        }}
      />
      <ClaudeSignInDialog
        target={signIn}
        onOpenChange={(open) => {
          if (!open) setSignIn(null)
        }}
        globalName={globalAccount?.name ?? claude?.global ?? null}
        globalWeek={weekPercent(globalAccount)}
        isGlobal={signInGlobal}
        onMakeGlobal={makeGlobal}
      />
      <AddAccountDialog
        open={adding}
        onOpenChange={setAdding}
        accounts={claude?.accounts ?? []}
        onAdded={(a) => {
          void refreshClaudeAccounts()
          setSignIn(target(a, true))
        }}
      />
      <RenameAccountDialog
        account={renaming}
        accounts={claude?.accounts ?? []}
        onClose={() => {
          setRenaming(null)
          void refreshClaudeAccounts()
        }}
      />
      <AccountColorDialog
        account={coloring}
        onClose={() => {
          setColoring(null)
          void refreshClaudeAccounts()
        }}
      />
      <RemoveAccountDialog
        account={removing}
        onClose={() => {
          setRemoving(null)
          void refreshClaudeAccounts()
        }}
      />
    </>
  )

  return {
    switching,
    status,
    clearStatus: () => setStatus(null),
    makeGlobal,
    openAdd: () => setAdding(true),
    openSignIn: (a) => setSignIn(target(a)),
    cardProps,
    dialogs,
  }
}

/** The line over the cards: a switch waiting for its windows, then the last action's outcome. */
export function AccountsStatus({
  data,
  actions,
}: {
  data: ClaudeAccounts
  actions: AccountActions
}) {
  const pending = data.claude?.pendingGlobal ?? null
  const waiting = pending
    ? waitingBanner(data.nameOf(pending.account), pending.waitingFor.length)
    : null
  const status = actions.status
  if (!waiting && !status) return null
  return (
    <div className="flex flex-col gap-1.5">
      {waiting ? (
        <p
          role="status"
          data-accounts-waiting=""
          className="rounded-lg bg-muted px-3 py-2 text-xs text-ink"
        >
          {waiting}
        </p>
      ) : null}
      {status && status.text !== waiting ? (
        <p
          role={status.tone === 'alert' ? 'alert' : 'status'}
          data-accounts-status={status.tone}
          className={cn(
            'rounded-lg px-3 py-2 text-xs',
            status.tone === 'alert'
              ? 'bg-alert-muted text-alert'
              : 'bg-muted text-ink',
          )}
        >
          {status.text}
        </p>
      ) : null}
    </div>
  )
}

/**
 * The account cards (§4.3.1) in a radio group whose value is the global
 * account: a grid in the full screen, a stack on the picker's page.
 */
export function AccountCards({
  data,
  actions,
  splits = false,
  layout = 'grid',
  time,
}: {
  data: ClaudeAccounts
  actions: AccountActions
  /** Show each account's week by project. */
  splits?: boolean
  layout?: 'grid' | 'stack'
  time: TimeStyle
}) {
  const claude = data.claude
  if (!claude) return null
  // The grid follows its pane (a container query), not the window.
  return (
    <div className="@container">
      <RadioGroup
        aria-label="Global account"
        value={claude.global}
        onValueChange={(value) => actions.makeGlobal(String(value))}
        className={
          layout === 'grid'
            ? 'grid-cols-1 @xl:grid-cols-2 @4xl:grid-cols-3'
            : 'grid-cols-1'
        }
      >
        {claude.accounts.map((a) => (
          <AccountCard
            key={a.id}
            account={a}
            isGlobal={a.id === claude.global}
            time={time}
            split={
              splits && data.splits !== null ? data.split(a.id) : undefined
            }
            {...actions.cardProps(a)}
          />
        ))}
      </RadioGroup>
    </div>
  )
}
