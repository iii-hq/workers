import { Ellipsis } from 'lucide-react'
import { Button } from '@/components/shadcn/button'
import {
  DropdownMenu,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { RadioGroupItem } from '@/components/shadcn/radio-group'
import { ChatMenuContent, ChatMenuItem } from '@/components/ui/ChatMenu'
import { FactMeter } from '@/components/ui/FactCard'
import { QuietIconButton } from '@/components/ui/QuietIconButton'
import { SplitBar } from '@/components/ui/SplitBar'
import type {
  AccountSplit,
  ClaudeAccount,
  UsageLimit,
} from '@/lib/backend/claude-accounts'
import {
  BUILT_IN_STAYS,
  COLOR,
  cardResetLine,
  claudePlan,
  FIVE_HOURS,
  GLOBAL_TAG,
  level,
  NOT_SIGNED_IN,
  noReading,
  percent,
  REMOVE,
  RENAME,
  SIGN_IN,
  SPLIT_TITLE,
  signInLabel,
  splitSpoken,
  splitView,
  THIS_WEEK,
  type TimeStyle,
  type UsageLevel,
} from '@/lib/claude-accounts-words'
import { CANCEL_SIGN_IN } from '@/lib/claude-sign-in-words'
import { cn } from '@/lib/utils'

/** Each tmux colour token's fill, spelled out so Tailwind sees every class. */
const DOT: Record<string, string> = {
  blue: 'bg-account-blue',
  green: 'bg-account-green',
  violet: 'bg-account-violet',
  orange: 'bg-account-orange',
  pink: 'bg-account-pink',
}

/** An account's colour as a dot, the tmux account row's (AccountLook); grey for a token it doesn't know. */
export function AccountDot({
  color,
  className,
}: {
  color: string
  className?: string
}) {
  return (
    <span
      aria-hidden
      data-account-color={color}
      className={cn(
        'inline-block size-2 shrink-0 rounded-full',
        DOT[color] ?? 'bg-account-gray',
        className,
      )}
    />
  )
}

const LEVEL_TEXT: Record<UsageLevel, string> = {
  normal: 'text-ink',
  high: 'text-warn',
  full: 'text-alert',
}

/** The meter's fill: the ink, amber from 80 %, red at 100 % (UsageWords.level). */
const LEVEL_ACCENT: Record<UsageLevel, string | undefined> = {
  normal: undefined,
  high: 'var(--color-warn)',
  full: 'var(--color-alert)',
}

/** "5 hours 38%" over its meter. */
export function UsageMeter({
  label,
  limit,
}: {
  label: string
  limit: UsageLimit | null
}) {
  const pct = limit ? percent(limit.usedPercent) : 0
  const tone = level(limit)
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-baseline gap-2 text-xs text-ink-faint">
        <span className="min-w-0 flex-1 truncate">{label}</span>
        <span className={cn('font-mono tabular-nums', LEVEL_TEXT[tone])}>
          {limit ? `${pct}%` : '–'}
        </span>
      </div>
      <FactMeter
        value={pct}
        accent={LEVEL_ACCENT[tone]}
        label={`${label} ${limit ? `${pct}%` : 'not read'}`}
      />
    </div>
  )
}

/**
 * "This week by project (from session tokens)" (D28): the split bar and its
 * legend, with "Outside the Chat" in the neutral grey; the note when the
 * week came only from outside, or when outside isn't measured.
 */
export function AccountSplitSection({ split }: { split: AccountSplit | null }) {
  const view = splitView(split)
  if (view.empty && !view.note) return null
  return (
    <div
      data-account-split=""
      className="flex flex-col gap-1.5 border-t border-border pt-2.5"
    >
      {view.items.length > 0 ? (
        <>
          <span className="text-xs text-ink-faint">{SPLIT_TITLE}</span>
          <SplitBar
            label={splitSpoken(view)}
            segments={view.items.map((item) => ({
              key: item.key,
              percent: item.percent,
              tone: item.tone,
            }))}
          />
          <ul className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-ink-faint">
            {view.items.map((item) => (
              <li
                key={item.key}
                data-split-item={item.tone}
                title={item.hint}
                className="flex min-w-0 items-center gap-1.5"
              >
                <SplitSwatch tone={item.tone} />
                <span className="truncate">{item.label}</span>
              </li>
            ))}
          </ul>
        </>
      ) : null}
      {view.note ? (
        <p data-split-note="" className="text-xs text-ink-faint">
          {view.note}
        </p>
      ) : null}
    </div>
  )
}

const SWATCH: Record<string, string> = {
  blue: 'bg-glyph-blue',
  purple: 'bg-glyph-purple',
  teal: 'bg-glyph-teal',
  green: 'bg-glyph-green',
  amber: 'bg-glyph-amber',
  rose: 'bg-glyph-rose',
  'no-project': 'bg-account-no-project',
  outside: 'bg-account-outside',
}

function SplitSwatch({ tone }: { tone: string }) {
  return (
    <span
      aria-hidden
      className={cn('size-1.5 shrink-0 rounded-xs', SWATCH[tone])}
    />
  )
}

export interface AccountCardProps {
  account: ClaudeAccount
  isGlobal: boolean
  time: TimeStyle
  /**
   * The week's split: undefined leaves the section out (the picker's page
   * before it is read), null is "no use this week".
   */
  split?: AccountSplit | null
  /** The radio is off while a switch is on its way. */
  switching?: boolean
  /** A sign-in for it is under way: the menu offers to cancel it. */
  signingIn?: boolean
  onSignIn?: () => void
  onCancelSignIn?: () => void
  onRename?: () => void
  onColor?: () => void
  onRemove?: () => void
  className?: string
}

/**
 * One Claude Code account (§4.3.1), the tmux account row as a card: the
 * radio that makes it global, its dot, name, "global" tag and plan, then
 * "5 hours" and "This week" with the reset line, then the week by project.
 * Not signed in: "Not signed in" and "Sign in". The radio belongs to the
 * caller's `RadioGroup` (its value is the global account).
 */
export function AccountCard({
  account: a,
  isGlobal,
  time,
  split,
  switching = false,
  signingIn = false,
  onSignIn,
  onCancelSignIn,
  onRename,
  onColor,
  onRemove,
  className,
}: AccountCardProps) {
  const plan = claudePlan(a.plan, a.tier)
  const missing = noReading(a)
  const reset = missing ? null : cardResetLine(a, time)
  const neverSignedIn = !a.signedIn && !a.tokenExpired
  const hasMenu = Boolean(onRename || onColor || onRemove || onSignIn)
  return (
    <section
      aria-label={a.name}
      data-account-card={a.id}
      data-global={isGlobal || undefined}
      className={cn(
        'flex min-w-0 flex-col gap-2.5 rounded-xl border px-4 py-3.5 text-sm',
        isGlobal ? 'border-input bg-card' : 'border-border',
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        <RadioGroupItem
          value={a.id}
          disabled={switching}
          aria-label={
            isGlobal ? `${a.name} is global` : `Make ${a.name} global`
          }
        />
        <AccountDot color={a.color} />
        <span className="min-w-0 truncate font-semibold text-ink">
          {a.name}
        </span>
        {isGlobal ? (
          <span className="shrink-0 rounded-full bg-muted px-1.5 text-meta text-ink-faint">
            {GLOBAL_TAG}
          </span>
        ) : null}
        <span className="flex-1" />
        {plan ? (
          <span className="shrink-0 text-xs text-ink-faint">{plan}</span>
        ) : null}
        {hasMenu ? (
          <AccountMenu
            account={a}
            signingIn={signingIn}
            onSignIn={onSignIn}
            onCancelSignIn={onCancelSignIn}
            onRename={onRename}
            onColor={onColor}
            onRemove={onRemove}
          />
        ) : null}
      </div>

      {neverSignedIn ? (
        <div className="flex items-center gap-3">
          <span className="min-w-0 flex-1 text-xs text-ink-faint">
            {NOT_SIGNED_IN}
          </span>
          {onSignIn ? (
            <Button size="sm" variant="secondary" onClick={onSignIn}>
              {SIGN_IN}
            </Button>
          ) : null}
        </div>
      ) : missing ? (
        <div className="flex items-center gap-3">
          <span className="min-w-0 flex-1 text-xs text-ink-faint">
            {missing}
          </span>
          {a.tokenExpired && onSignIn ? (
            <Button size="sm" variant="secondary" onClick={onSignIn}>
              {signInLabel(a)}
            </Button>
          ) : null}
        </div>
      ) : (
        <>
          <UsageMeter label={FIVE_HOURS} limit={a.usage?.fiveHour ?? null} />
          <UsageMeter label={THIS_WEEK} limit={a.usage?.week ?? null} />
          {reset ? (
            <span className="text-xs text-ink-faint">{reset}</span>
          ) : null}
        </>
      )}

      {split !== undefined && !neverSignedIn ? (
        <AccountSplitSection split={split} />
      ) : null}
    </section>
  )
}

/** The tmux account menu: Rename…, Color…, Sign in (again), Remove…. */
function AccountMenu({
  account: a,
  signingIn,
  onSignIn,
  onCancelSignIn,
  onRename,
  onColor,
  onRemove,
}: {
  account: ClaudeAccount
  signingIn: boolean
  onSignIn?: () => void
  onCancelSignIn?: () => void
  onRename?: () => void
  onColor?: () => void
  onRemove?: () => void
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={`More actions for ${a.name}`}
        render={<QuietIconButton className="-my-1" />}
      >
        <Ellipsis aria-hidden className="size-4" />
      </DropdownMenuTrigger>
      <ChatMenuContent align="end" sideOffset={6} className="min-w-44">
        {onRename ? (
          <ChatMenuItem onClick={onRename}>{RENAME}</ChatMenuItem>
        ) : null}
        {onColor ? (
          <ChatMenuItem onClick={onColor}>{COLOR}</ChatMenuItem>
        ) : null}
        {signingIn && onCancelSignIn ? (
          <ChatMenuItem onClick={onCancelSignIn}>{CANCEL_SIGN_IN}</ChatMenuItem>
        ) : onSignIn ? (
          <ChatMenuItem onClick={onSignIn}>{signInLabel(a)}</ChatMenuItem>
        ) : null}
        {onRemove ? (
          <>
            <DropdownMenuSeparator className="my-1.5" />
            <ChatMenuItem
              variant="destructive"
              disabled={a.isDefault}
              title={a.isDefault ? BUILT_IN_STAYS : undefined}
              onClick={onRemove}
            >
              {REMOVE}
            </ChatMenuItem>
          </>
        ) : null}
      </ChatMenuContent>
    </DropdownMenu>
  )
}
