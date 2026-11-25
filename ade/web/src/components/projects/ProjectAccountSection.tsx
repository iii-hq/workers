import { ChevronDown } from 'lucide-react'
import { type Ref, useId } from 'react'
import { AccountDot } from '@/components/chat/AccountCard'
import {
  DropdownMenu,
  DropdownMenuRadioGroup,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { RadioGroup, RadioGroupItem } from '@/components/shadcn/radio-group'
import { ChatMenuContent, ChatMenuRadioItem } from '@/components/ui/ChatMenu'
import type { ClaudeAccounts } from '@/lib/backend/claude-accounts'
import type { ProjectAccount } from '@/lib/backend/projects'
import {
  accountWeek,
  FOLLOW_GLOBAL,
  globalNow,
  mostRoom,
  OTHER_PROVIDERS_IGNORE,
  PIN_TO,
  PROJECT_ACCOUNT_LABEL,
  usable,
} from '@/lib/claude-accounts-words'

/** The account a new pin starts on: the one with the most room, else the first usable one. */
export function defaultPin(data: ClaudeAccounts): string | null {
  const claude = data.claude
  if (!claude) return null
  return (
    mostRoom(claude, claude.global)?.id ??
    claude.accounts.find((a) => a.id !== claude.global && usable(a))?.id ??
    claude.accounts.find(usable)?.id ??
    null
  )
}

/**
 * The project dialog's "Claude Code account" (§4.3.2): follow the global
 * account ("Work now"), or pin to one, chosen from a menu of the accounts
 * with their week (signed-out ones disabled). Other providers ignore it.
 */
export function ProjectAccountSection({
  value,
  onChange,
  data,
  error,
  firstRef,
}: {
  value: ProjectAccount
  onChange: (next: ProjectAccount) => void
  data: ClaudeAccounts
  error?: string
  /** The first radio, for the dialog opened on this section. */
  firstRef?: Ref<HTMLElement>
}) {
  const ids = useId()
  const claude = data.claude
  const global = data.account(claude?.global)
  const pinnedId = value.mode === 'pinned' ? value.account_id : null
  const pinned = data.account(pinnedId)
  const fallback = defaultPin(data)
  const shown = pinned ?? data.account(fallback)
  return (
    <section
      data-project-account=""
      aria-labelledby={`${ids}-label`}
      className="flex flex-col gap-2"
    >
      <span id={`${ids}-label`} className="text-body font-medium text-ink">
        {PROJECT_ACCOUNT_LABEL}
      </span>
      <RadioGroup
        aria-labelledby={`${ids}-label`}
        value={value.mode}
        onValueChange={(mode) => {
          if (mode === 'global') onChange({ mode: 'global' })
          else {
            const id = pinnedId ?? fallback
            if (id) onChange({ mode: 'pinned', account_id: id })
          }
        }}
      >
        <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
          <RadioGroupItem ref={firstRef} id={`${ids}-global`} value="global" />
          <label htmlFor={`${ids}-global`}>{FOLLOW_GLOBAL}</label>
          {global ? (
            <span className="flex items-center gap-1.5 text-xs text-ink-faint">
              <AccountDot color={global.color} />
              {globalNow(global.name)}
            </span>
          ) : null}
        </div>
        <div className="flex min-h-8 flex-wrap items-center gap-2 text-sm text-ink">
          <RadioGroupItem
            id={`${ids}-pinned`}
            value="pinned"
            disabled={!pinnedId && !fallback}
          />
          <label htmlFor={`${ids}-pinned`}>{PIN_TO}</label>
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label={`${PIN_TO}: ${shown ? accountWeek(shown) : 'choose an account'}`}
              disabled={!claude || claude.accounts.length === 0}
              data-project-account-pin=""
              render={
                <button
                  type="button"
                  className="flex h-8 max-w-full items-center gap-2 rounded-lg border border-input bg-card px-2.5 text-body text-ink-faint hover:bg-surface-hover focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50 disabled:opacity-50"
                />
              }
            >
              {shown ? <AccountDot color={shown.color} /> : null}
              <span className="truncate">
                {shown ? accountWeek(shown) : (pinnedId ?? '—')}
              </span>
              <ChevronDown aria-hidden className="size-4 shrink-0" />
            </DropdownMenuTrigger>
            <ChatMenuContent align="start" sideOffset={6} className="min-w-56">
              <DropdownMenuRadioGroup
                value={pinnedId ?? ''}
                onValueChange={(id) => {
                  if (typeof id === 'string' && id) {
                    onChange({ mode: 'pinned', account_id: id })
                  }
                }}
              >
                {claude?.accounts.map((a) => (
                  <ChatMenuRadioItem
                    key={a.id}
                    value={a.id}
                    disabled={!usable(a)}
                    data-account-option={a.id}
                  >
                    <AccountDot color={a.color} />
                    {accountWeek(a)}
                  </ChatMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </ChatMenuContent>
          </DropdownMenu>
        </div>
      </RadioGroup>
      <p className="pl-6 text-xs text-ink-faint">{OTHER_PROVIDERS_IGNORE}</p>
      {error ? (
        <p role="alert" className="text-xs text-alert">
          {error}
        </p>
      ) : null}
    </section>
  )
}
