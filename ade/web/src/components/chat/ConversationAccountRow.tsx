import type { Ref } from 'react'
import { useId } from 'react'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from '@/components/shadcn/select'
import { FormSelectTrigger } from '@/components/ui/FormDialog'
import {
  type ClaudeAccounts,
  FOLLOW_GLOBAL as FOLLOW_GLOBAL_ID,
} from '@/lib/backend/claude-accounts'
import { type ChatProject, parseProjectAccount } from '@/lib/backend/projects'
import {
  ACCOUNT_ROW_LABEL,
  type AccountResolution,
  defaultChoiceLabel,
  globalChoiceLabel,
  resolutionText,
  usable,
} from '@/lib/claude-accounts-words'
import { conversationResolution } from '@/lib/conversation-account'
import { ACCOUNT_DEFAULT } from '@/lib/conversation-details'
import { AccountDot } from './AccountCard'

/**
 * Conversation details' "Claude Code account" row (§4.3.5): the resolution
 * as text, then a select of the project's account (or the global without a
 * project) and each account, which pins it for this conversation. A side
 * chat reads the text only: its account is locked (D2).
 */
export function ConversationAccountRow({
  value,
  onChange,
  project,
  data,
  readOnly = false,
  triggerRef,
  resolution: given,
}: {
  /** The draft's `claude_account`: `ACCOUNT_DEFAULT`, `"global"` or an id. */
  value: string
  onChange: (next: string) => void
  project: Pick<ChatProject, 'name' | 'account'> | null
  data: ClaudeAccounts
  readOnly?: boolean
  triggerRef?: Ref<HTMLButtonElement>
  /**
   * The account the engine runs it on when the choice doesn't decide it (a
   * side chat's lock, a sub-agent's parent): the text reads it.
   */
  resolution?: AccountResolution | null
}) {
  const ids = useId()
  const claude = data.claude
  const global = claude?.global ?? null
  const globalName = global ? data.nameOf(global) : null
  const projectAccount = project ? parseProjectAccount(project.account) : null
  const resolution =
    given ?? conversationResolution({ choice: value, project, global })
  const text = resolutionText(
    resolution,
    data.nameOf,
    value === FOLLOW_GLOBAL_ID,
  )
  const shown = data.account(resolution.account)

  const items: {
    value: string
    label: string
    color?: string
    disabled?: boolean
  }[] = [
    {
      value: ACCOUNT_DEFAULT,
      label: defaultChoiceLabel({
        hasProject: project !== null,
        projectAccount,
        globalName,
        nameOf: data.nameOf,
      }),
    },
  ]
  // Following the global while the project pins another account.
  if (project && projectAccount?.mode === 'pinned') {
    items.push({
      value: FOLLOW_GLOBAL_ID,
      label: globalChoiceLabel(globalName),
    })
  }
  for (const a of claude?.accounts ?? []) {
    items.push({
      value: a.id,
      label: a.name,
      color: a.color,
      disabled: !usable(a) && a.id !== value,
    })
  }
  if (!items.some((item) => item.value === value)) {
    // A pin to an account that is gone: shown as it is until changed.
    items.push({ value, label: value })
  }

  return (
    <div data-conversation-account="" className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-3">
        <span
          id={`${ids}-label`}
          className="flex-1 text-body font-medium text-foreground"
        >
          {ACCOUNT_ROW_LABEL}
        </span>
        {readOnly ? null : (
          <Select
            value={value}
            onValueChange={(next) => onChange(String(next))}
            items={items.map(({ value: v, label }) => ({ value: v, label }))}
          >
            <FormSelectTrigger
              ref={triggerRef}
              aria-labelledby={`${ids}-label`}
              className="h-9 min-w-44"
            >
              <SelectValue />
            </FormSelectTrigger>
            <SelectContent align="end">
              {items.map((item) => (
                <SelectItem
                  key={item.value}
                  value={item.value}
                  disabled={item.disabled}
                >
                  {item.color ? <AccountDot color={item.color} /> : null}
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>
      <p
        data-account-resolution={resolution.source}
        className="flex items-center gap-1.5 text-xs text-ink-faint"
      >
        {shown ? <AccountDot color={shown.color} /> : null}
        {text}
      </p>
    </div>
  )
}
