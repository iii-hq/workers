import * as SelectPrimitive from '@radix-ui/react-select'
import {
  Check,
  ChevronDown,
  ListTodo,
  MessageSquare,
  Repeat,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { MODES, type Mode } from '@/types/chat'

interface ModePickerProps {
  value: Mode
  onChange: (next: Mode) => void
  className?: string
}

const MODE_META: Record<
  Mode,
  { icon: typeof Repeat; iconClassName: string; label: string }
> = {
  agent: { icon: Repeat, iconClassName: 'text-ink-faint', label: 'agent' },
  plan: { icon: ListTodo, iconClassName: 'text-warn', label: 'plan' },
  ask: { icon: MessageSquare, iconClassName: 'text-ok', label: 'ask' },
}

function ModeIcon({ mode, size = 14 }: { mode: Mode; size?: number }) {
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
          'inline-flex items-center justify-between gap-x-2 border border-rule bg-bg px-3 h-9 text-ink font-mono text-[13px] lowercase focus:outline-none focus:border-ink data-[state=open]:border-ink transition-colors',
          className,
        )}
      >
        <span className="inline-flex items-center gap-2 min-w-0">
          <ModeIcon mode={value} />
          <SelectPrimitive.Value>{meta.label}</SelectPrimitive.Value>
        </span>

        <SelectPrimitive.Icon asChild>
          <ChevronDown size={12} aria-hidden />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>

      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          sideOffset={4}
          className={cn(
            'z-50 min-w-[var(--radix-select-trigger-width)] overflow-hidden border border-rule bg-bg text-ink font-mono text-[13px] lowercase shadow-lg',
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
                  'relative flex items-center gap-2 pl-7 pr-3 py-1.5 cursor-pointer outline-none select-none',
                  'data-[highlighted]:bg-rule data-[highlighted]:text-ink',
                  'data-[state=checked]:text-ink',
                )}
              >
                <SelectPrimitive.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2 text-ink">
                  <Check size={12} aria-hidden />
                </SelectPrimitive.ItemIndicator>
                <ModeIcon mode={m.id} />
                <SelectPrimitive.ItemText>{m.label}</SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  )
}
