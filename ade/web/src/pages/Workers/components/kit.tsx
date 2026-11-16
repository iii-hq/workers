/**
 * The Workers page's small parts, on the shadcn primitives: a notice, an
 * empty state, a text input and a search box, a field label and a confirm
 * dialog. Each keeps the prop shape of the console component it replaces
 * so the page reads the same; only the surface changes — Inter, neutral
 * borders, `rounded-lg`/`rounded-xl`, no shadows, room around the text.
 */

import {
  CircleAlert,
  CircleCheck,
  Info,
  type LucideIcon,
  Search,
  TriangleAlert,
  X,
} from 'lucide-react'
import * as React from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { Input } from '@/components/shadcn/input'
import { cn } from '@/lib/utils'

/* ── Notice ──────────────────────────────────────────────────────────── */

export type NoticeVariant = 'info' | 'success' | 'warn' | 'alert'

const NOTICE: Record<
  NoticeVariant,
  { icon: LucideIcon; box: string; icon_: string; title: string }
> = {
  info: {
    icon: Info,
    box: 'bg-muted outline-border  ',
    icon_: 'text-muted-foreground ',
    title: 'text-foreground ',
  },
  success: {
    icon: CircleCheck,
    box: 'bg-ok-muted outline-ok/25',
    icon_: 'text-ok',
    title: 'text-ok',
  },
  warn: {
    icon: TriangleAlert,
    box: 'bg-warn-muted outline-warn/25',
    icon_: 'text-warn-strong',
    title: 'text-warn-strong',
  },
  alert: {
    icon: CircleAlert,
    box: 'bg-alert-muted outline-alert/25',
    icon_: 'text-alert-strong',
    title: 'text-alert-strong',
  },
}

export interface NoticeProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: NoticeVariant
  headline: React.ReactNode
  detail?: React.ReactNode
  /** Trailing slot for a retry or dismiss button; never text. */
  action?: React.ReactNode
}

/** A status with copy: a tinted, bordered block with the headline in the
 status colour and the detail below it. `role` passes through. */
export function Notice({
  variant = 'info',
  headline,
  detail,
  action,
  className,
  ...props
}: NoticeProps) {
  const tone = NOTICE[variant]
  const Icon = tone.icon
  return (
    <div
      className={cn(
        'wk-notice flex items-start gap-3 rounded-xl px-4 py-3.5 font-inter outline-1 -outline-offset-1',
        tone.box,
        className,
      )}
      {...props}
    >
      <Icon aria-hidden className={cn('mt-0.5 size-4 shrink-0', tone.icon_)} />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div
          className={cn(
            'text-body font-medium leading-5 [overflow-wrap:anywhere]',
            tone.title,
          )}
        >
          {headline}
        </div>
        {detail ? (
          // A reading measure, not the pane's width: a notice in a wide pane
          // stays a paragraph a person can read (≈77 characters a line).
          <div className="max-w-[64ch] text-body leading-5 text-muted-foreground [overflow-wrap:anywhere]">
            {detail}
          </div>
        ) : null}
      </div>
      {action ? (
        <div className="ml-auto flex shrink-0 items-center gap-1.5 self-center">
          {action}
        </div>
      ) : null}
    </div>
  )
}

/* ── Empty ───────────────────────────────────────────────────────────── */

export interface EmptyProps {
  icon?: React.ComponentType<{ className?: string }>
  title: string
  description: string
  action?: { label: string; onClick: () => void }
  /** Inside a card or a list: a bordered row instead of a centred block. */
  compact?: boolean
  className?: string
}

export function Empty({
  icon: Icon,
  title,
  description,
  action,
  compact,
  className,
}: EmptyProps) {
  if (compact)
    return (
      <div
        className={cn(
          'flex items-start gap-3 rounded-xl border border-border px-4 py-3.5 font-inter',
          className,
        )}
      >
        {Icon ? (
          <Icon
            aria-hidden
            className="mt-0.5 size-4 shrink-0 text-muted-foreground"
          />
        ) : null}
        <div className="min-w-0 flex-1">
          <p className="text-body font-medium leading-5 text-foreground">
            {title}
          </p>
          <p className="text-body leading-5 text-muted-foreground">
            {description}
          </p>
          {action ? (
            <Button
              variant="outline"
              size="sm"
              className="wk-press mt-3 font-inter"
              onClick={action.onClick}
            >
              {action.label}
            </Button>
          ) : null}
        </div>
      </div>
    )
  return (
    <div
      className={cn(
        'flex flex-1 flex-col items-center justify-center gap-2 px-6 py-16 text-center font-inter',
        className,
      )}
    >
      {Icon ? (
        <span className="mb-1 flex size-10 items-center justify-center rounded-xl border border-border text-muted-foreground">
          <Icon aria-hidden className="size-4" />
        </span>
      ) : null}
      <p className="text-sm font-medium leading-5 text-foreground">{title}</p>
      <p className="max-w-[40ch] text-body leading-5 text-muted-foreground">
        {description}
      </p>
      {action ? (
        <Button
          variant="outline"
          className="wk-press mt-3 font-inter"
          onClick={action.onClick}
        >
          {action.label}
        </Button>
      ) : null}
    </div>
  )
}

/* ── Text input ──────────────────────────────────────────────────────── */

export interface TextInputProps
  extends Omit<React.ComponentProps<'input'>, 'onChange' | 'value'> {
  value: string
  onChange: (next: string) => void
}

/** A text field: the shadcn Input with the console's `(next: string)` change. */
export const TextInput = React.forwardRef<HTMLInputElement, TextInputProps>(
  ({ className, value, onChange, ...rest }, ref) => (
    <Input
      ref={ref}
      value={value}
      onChange={(event) => onChange(event.currentTarget.value)}
      className={cn('wk-input', className)}
      {...rest}
    />
  ),
)
TextInput.displayName = 'TextInput'

/* ── Search box ──────────────────────────────────────────────────────── */

export interface SearchBoxProps
  extends Omit<
    React.ComponentProps<'input'>,
    'onChange' | 'value' | 'type' | 'className'
  > {
  value: string
  onChange: (next: string) => void
  /** Applies to the wrapper; everything else lands on the input. */
  className?: string
}

/** A search field: leading magnifier, a clear button while non-empty,
 Escape clears (and stops there so an enclosing dialog stays open). */
export const SearchBox = React.forwardRef<HTMLInputElement, SearchBoxProps>(
  (
    { value, onChange, className, onKeyDown, 'aria-label': ariaLabel, ...rest },
    ref,
  ) => {
    const inner = React.useRef<HTMLInputElement | null>(null)
    const setRef = React.useCallback(
      (node: HTMLInputElement | null) => {
        inner.current = node
        if (typeof ref === 'function') ref(node)
        else if (ref) ref.current = node
      },
      [ref],
    )
    return (
      <div className={cn('relative min-w-0', className)}>
        <Search
          aria-hidden
          className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-ink-ghost"
        />
        <Input
          ref={setRef}
          {...rest}
          type="search"
          value={value}
          aria-label={ariaLabel ?? rest.placeholder}
          onChange={(event) => onChange(event.currentTarget.value)}
          className="wk-input pl-9 pr-9 [&::-webkit-search-cancel-button]:hidden"
          onKeyDown={(event) => {
            onKeyDown?.(event)
            if (event.defaultPrevented) return
            if (event.key === 'Escape' && value) {
              event.preventDefault()
              event.stopPropagation()
              onChange('')
            }
          }}
        />
        {value ? (
          <button
            type="button"
            aria-label="Clear"
            onClick={() => {
              onChange('')
              inner.current?.focus({ preventScroll: true })
            }}
            className="absolute right-1.5 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-ink-ghost outline-none transition-colors duration-150 hover:bg-surface-hover/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            <span
              className="pointer-events-none absolute left-1/2 top-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            />
            <X className="size-4" aria-hidden />
          </button>
        ) : null}
      </div>
    )
  },
)
SearchBox.displayName = 'SearchBox'

/* ── Field label ─────────────────────────────────────────────────────── */

export function FieldLabel({
  className,
  as: Tag = 'span',
  ...props
}: React.HTMLAttributes<HTMLElement> & {
  as?: 'span' | 'label' | 'dt' | 'h3'
  htmlFor?: string
}) {
  return (
    <Tag
      className={cn(
        'font-inter text-xs font-medium leading-4 text-muted-foreground',
        className,
      )}
      {...props}
    />
  )
}

/* ── Confirm dialog ──────────────────────────────────────────────────── */

export interface ConfirmOptions {
  title: string
  description?: React.ReactNode
  /** Lines listed under the description, e.g. the workers at stake. */
  details?: readonly string[]
  confirmLabel?: string
  cancelLabel?: string
  /** `danger` paints the confirm control in the alert colour. */
  tone?: 'default' | 'danger'
}

function ConfirmBox({
  open,
  options,
  onSettle,
}: {
  open: boolean
  options: ConfirmOptions
  onSettle: (confirmed: boolean) => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  // A confirmed dialog hands focus to whatever the caller reveals next (a
  // review region, a result notice); only a cancel returns it to the trigger.
  const confirmed = React.useRef(false)
  const settle = (next: boolean) => {
    confirmed.current = next
    onSettle(next)
  }
  const {
    title,
    description,
    details,
    confirmLabel = 'Continue',
    cancelLabel = 'Cancel',
    tone = 'default',
  } = options
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) settle(false)
      }}
    >
      <DialogContent
        role="alertdialog"
        initialFocus={cancelRef}
        finalFocus={() => !confirmed.current}
        className="wk-surface wk-dialog-box gap-0 p-0 [--dialog-max-width:28rem]"
      >
        <div className="flex flex-col gap-2 px-6 pt-6 pb-5">
          <DialogTitle className="pr-8 font-inter text-base font-semibold leading-6 text-foreground">
            {title}
          </DialogTitle>
          {description ? (
            <DialogDescription className="font-inter text-sm leading-[22px] text-muted-foreground">
              {description}
            </DialogDescription>
          ) : null}
          {details && details.length > 0 ? (
            <ul className="mt-2 flex flex-col gap-1 rounded-xl border border-border px-3.5 py-3 text-body leading-5 text-foreground">
              {details.map((line, index) => (
                <li
                  // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional and may repeat
                  key={`${index}:${line}`}
                  className="truncate"
                >
                  {line}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        <div className="flex flex-wrap justify-end gap-2 border-t border-border px-6 py-4">
          <Button
            ref={cancelRef}
            type="button"
            variant="outline"
            size="lg"
            className="wk-press font-inter"
            onClick={() => settle(false)}
          >
            {cancelLabel}
          </Button>
          <Button
            type="button"
            variant={tone === 'danger' ? 'destructive' : 'default'}
            size="lg"
            className="wk-press font-inter"
            onClick={() => settle(true)}
          >
            {confirmLabel}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}

/**
 * `window.confirm` shaped around the page's dialog: render `dialog` once,
 * then `await confirm({ title, … })`. Escape and Cancel resolve `false`;
 * a second call while one is open cancels the first; unmounting cancels
 * whatever is open.
 */
export function useConfirm(): {
  confirm: (options: ConfirmOptions) => Promise<boolean>
  dialog: React.ReactNode
} {
  const [options, setOptions] = React.useState<ConfirmOptions | null>(null)
  const resolveRef = React.useRef<((confirmed: boolean) => void) | null>(null)
  const settle = React.useCallback((confirmed: boolean) => {
    resolveRef.current?.(confirmed)
    resolveRef.current = null
    setOptions(null)
  }, [])
  const confirm = React.useCallback(
    (next: ConfirmOptions) =>
      new Promise<boolean>((resolve) => {
        resolveRef.current?.(false)
        resolveRef.current = resolve
        setOptions(next)
      }),
    [],
  )
  React.useEffect(
    () => () => {
      resolveRef.current?.(false)
      resolveRef.current = null
    },
    [],
  )
  const dialog = options ? (
    <ConfirmBox open options={options} onSettle={settle} />
  ) : null
  return { confirm, dialog }
}
