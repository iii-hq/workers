import * as SelectPrimitive from '@radix-ui/react-select'
import {
  Bot,
  Check,
  ChevronDown,
  type LucideIcon,
  MessageSquare,
} from 'lucide-react'
import { SheetOptionList } from '@/components/ui/SheetNavigation'
import { cn } from '@/lib/utils'
import { MODES, type Mode } from '@/types/chat'

interface ModePickerProps {
  value: Mode
  onChange: (next: Mode) => void
  className?: string
}

const MODE_META: Record<
  Mode,
  { icon: LucideIcon; iconClassName: string; label: string }
> = {
  agent: { icon: Bot, iconClassName: 'text-ink-faint', label: 'Agent' },
  ask: { icon: MessageSquare, iconClassName: 'text-ok', label: 'Ask' },
}

function ModeIcon({ mode, size = 16 }: { mode: Mode; size?: number }) {
  const meta = MODE_META[mode]
  const Icon = meta.icon
  return <Icon size={size} className={meta.iconClassName} aria-hidden />
}

export function ModePicker({ value, onChange, className }: ModePickerProps) {
  const meta = MODE_META[value]

  return (
    <SelectPrimitive.Root
      value={value}
      onValueChange={(next) => onChange(next as Mode)}
    >
      <SelectPrimitive.Trigger
        aria-label="agent mode"
        className={cn(
          'inline-flex h-12 items-center justify-between gap-x-2 rounded-sm border border-transparent bg-transparent px-3 font-sans text-base text-ink-faint hover:bg-surface-hover hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus data-[state=open]:bg-surface data-[state=open]:text-ink sm:h-9 sm:text-[13px]',
          className,
        )}
      >
        <span className="inline-flex min-w-0 items-center gap-2">
          <ModeIcon mode={value} />
          <SelectPrimitive.Value>{meta.label}</SelectPrimitive.Value>
        </span>

        <SelectPrimitive.Icon asChild>
          <ChevronDown size={16} aria-hidden />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>

      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          className={cn(
            'z-50 min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-md border border-rule-2 bg-panel-raised font-sans text-base text-ink shadow-floating sm:text-[13px]',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
          )}
        >
          <SelectPrimitive.Viewport className="p-1">
            {MODES.map((m) => (
              <SelectPrimitive.Item
                key={m.id}
                value={m.id}
                className={cn(
                  'relative flex min-h-12 cursor-pointer select-none items-center gap-2 py-2 pr-3 pl-7 outline-none sm:min-h-0 sm:py-1.5',
                  'rounded-xs data-[highlighted]:bg-surface-hover data-[highlighted]:text-ink',
                  'data-[state=checked]:text-ink',
                )}
              >
                <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
                  <Check size={16} aria-hidden />
                </SelectPrimitive.ItemIndicator>
                <ModeIcon mode={m.id} />
                <SelectPrimitive.ItemText>
                  {MODE_META[m.id].label}
                </SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}

interface ModePickerPanelProps {
  value: Mode
  onChange: (next: Mode) => void
  disabled?: boolean
  className?: string
}

/** Inline mode choices for an existing navigable sheet. */
export function ModePickerPanel({
  value,
  onChange,
  disabled,
  className,
}: ModePickerPanelProps) {
  return (
    <SheetOptionList
      value={value}
      onChange={onChange}
      disabled={disabled}
      className={className}
      options={MODES.map((mode) => ({
        value: mode.id,
        label: MODE_META[mode.id].label,
        icon: <ModeIcon mode={mode.id} size={18} />,
      }))}
    />
  )
}
