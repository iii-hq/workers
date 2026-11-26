import { TriangleAlert } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/shadcn/button'
import { Hint } from '@/components/ui/Hint'
import { getDefaultBackend } from '@/lib/backend'
import {
  type ClaudeAccounts,
  dismissSuggestion,
  useClaudeAccounts,
} from '@/lib/backend/claude-accounts'
import {
  FOOTER_TITLE,
  mostRoom,
  NOT_NOW,
  ringLine,
  ringsLabel,
  ringsOrder,
  suggestionBody,
  suggestionTitle,
  suggestionUse,
} from '@/lib/claude-accounts-words'
import { bundledProviderMark } from '@/lib/provider-marks'
import { AccountDot } from './AccountCard'
import {
  type AccountActions,
  AccountsStatus,
  useAccountActions,
} from './AccountCards'
import { ClaudeAccountsDialog } from './ClaudeAccountsDialog'
import { requestClaudeAccountsDialog } from './claude-accounts-dialog-request'
import { ProviderIcon } from './ProviderIcon'
import { UsageRing } from './UsageRing'

/**
 * tmux's suggestion card (§4.3.3), when its rules are in suggest mode and
 * have one: "Work, the global account, is at 91%", who has room, "Make
 * Personal global" (or "Sign in to Personal" first) and "Not now". Shown
 * once, above the footer, never in every conversation.
 */
export function SuggestionCard({
  data,
  actions,
}: {
  data: ClaudeAccounts
  actions: AccountActions
}) {
  const [dismissed, setDismissed] = useState<string | null>(null)
  const claude = data.claude
  const s = claude?.suggestion ?? null
  if (!claude || !s || claude.rules.mode !== 'suggest' || dismissed === s.id) {
    return null
  }
  const to = data.account(s.to)
  if (!to) return null
  const signIn = !to.signedIn
  const isMostRoom = mostRoom(claude, s.from)?.id === to.id
  return (
    <div
      role="status"
      data-suggestion={s.id}
      className="flex flex-col gap-2 rounded-lg bg-warn-muted px-3 py-2.5"
    >
      <div className="flex gap-2">
        <TriangleAlert
          aria-hidden
          className="mt-px size-4 shrink-0 text-warn"
        />
        <div className="flex min-w-0 flex-col gap-0.5">
          <p className="text-body font-semibold text-ink">
            {suggestionTitle(data.account(s.from), s.from, s.reason)}
          </p>
          <p className="text-xs text-ink-faint">
            {suggestionBody(to, s.to, isMostRoom, s.following, true)}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 pl-6">
        <Button
          size="sm"
          data-suggestion-use={to.id}
          disabled={actions.switching}
          onClick={() =>
            signIn ? actions.openSignIn(to) : actions.makeGlobal(to.id)
          }
        >
          <AccountDot color={to.color} />
          {suggestionUse(to.name, signIn)}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            setDismissed(s.id)
            dismissSuggestion(s.id).catch(() => setDismissed(null))
          }}
        >
          {NOT_NOW}
        </Button>
      </div>
    </div>
  )
}

/**
 * The sidebar's usage footer (§4.3.3, D137, D178): one button, the Claude
 * Code mark and a ring per account's week, the global account first, no
 * text; the button's name says every account and a ring's tooltip its own
 * line. The suggestion card above it. Claude Code only; a click opens the
 * accounts.
 */
export function UsageFooter() {
  const realBackend = getDefaultBackend().id === 'real'
  const data = useClaudeAccounts(realBackend)
  const actions = useAccountActions(data)
  const [ownOpen, setOwnOpen] = useState(false)
  const claude = data.claude
  if (data.gate !== 'full' || !claude || claude.accounts.length === 0) {
    return null
  }
  const open = () => {
    if (!requestClaudeAccountsDialog()) setOwnOpen(true)
  }
  return (
    <section
      aria-label={FOOTER_TITLE}
      data-usage-footer=""
      className="flex shrink-0 flex-col gap-2 border-t border-border px-2 pt-2.5 pb-3"
    >
      <AccountsStatus data={data} actions={actions} />
      <SuggestionCard data={data} actions={actions} />
      <button
        type="button"
        data-usage-rings=""
        aria-label={ringsLabel(claude.accounts, claude.global)}
        onClick={open}
        className="flex min-h-11 items-center gap-2.5 self-start rounded-xl px-1.5 py-1 hover:bg-surface-hover focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50"
      >
        <ProviderIcon
          iconSvg={bundledProviderMark('claude-code')}
          label="Claude Code"
          providerId="claude-code"
          className="size-5"
        />
        {ringsOrder(claude.accounts, claude.global).map((a) => (
          <Hint
            key={a.id}
            side="top"
            label={ringLine(a, a.id === claude.global)}
          >
            <span aria-hidden data-footer-account={a.id} className="flex">
              <UsageRing week={a.usage?.week} color={a.color} />
            </span>
          </Hint>
        ))}
      </button>
      {actions.dialogs}
      {ownOpen ? <ClaudeAccountsDialog open onOpenChange={setOwnOpen} /> : null}
    </section>
  )
}
