// A stand-in for the console's runtime module (its import map serves the real one).
// Only the host boundary is substituted: the page, hooks, and model code stay real.
import {
  createContext,
  type HTMLAttributes,
  type ReactElement,
  type ReactNode,
  useContext,
  useEffect,
  useState,
} from 'react'

type P = HTMLAttributes<HTMLElement> & { children?: ReactNode; [k: string]: unknown }
const cb = (f: unknown) => f as (v: string) => void
const strip = ({
  variant,
  size,
  tone,
  tooltip,
  asChild,
  interactive,
  selected,
  inset,
  density,
  label,
  onClose,
  as,
  leading,
  trailing,
  description,
  headline,
  detail,
  action,
  icon,
  end,
  ...rest
}: P) => ({
  rest,
  variant,
  tone,
  selected,
  label,
  as,
  leading,
  trailing,
  description,
  headline,
  detail,
  action,
  icon,
  end,
  onClose,
  asChild,
})
const passthrough = (tag: 'div' | 'span' | 'section') => (p: P) => {
  const { rest } = strip(p)
  const T = tag
  return <T {...(rest as object)} />
}

export const PageShell = ({ ref: _ref, ...p }: P) => <div {...(strip(p).rest as object)} />
export const PageMain = passthrough('div')
export const PageBody = ({ side: _side, ...p }: P) => <div {...(strip(p).rest as object)} />
export const PageSidebar = ({ label, header, children }: P) => (
  <aside aria-label={label as string}>
    {header as ReactNode}
    {children}
  </aside>
)
export const PageHeader = ({ title, description, actions, children, onClose }: P) => (
  <header>
    <h1>{title as ReactNode}</h1>
    <span data-slot="description">{description as ReactNode}</span>
    {children}
    {actions as ReactNode}
    {onClose ? <button type="button" aria-label="Close" onClick={onClose as () => void} /> : null}
  </header>
)
export const Card = passthrough('div')
export const Panel = passthrough('section')
export const PanelHeader = passthrough('div')
export const PanelBody = passthrough('div')
export const Badge = ({ variant, children }: P) => (
  <span data-badge data-variant={variant as string}>
    {children}
  </span>
)
export const CardHeader = passthrough('div')
export const CardBody = passthrough('div')
/** As the console's: controlled or not, the trigger toggles it, the content stays mounted (`aria-hidden` when closed). */
const FoldCtx = createContext<{ open: boolean; toggle: () => void }>({ open: false, toggle: () => undefined })
export const CollapsibleCard = ({ open, defaultOpen, onOpenChange, disabled: _d, ...p }: P) => {
  const [own, setOwn] = useState(!!defaultOpen)
  const shown = open === undefined ? own : !!open
  const toggle = () => {
    if (open === undefined) setOwn(!shown)
    ;(onOpenChange as ((o: boolean) => void) | undefined)?.(!shown)
  }
  return (
    <FoldCtx.Provider value={{ open: shown, toggle }}>
      <div data-state={shown ? 'open' : 'closed'} {...(strip(p).rest as object)} />
    </FoldCtx.Provider>
  )
}
export const CollapsibleCardTrigger = ({ onClick, ...p }: P) => {
  const f = useContext(FoldCtx)
  return (
    <button
      type="button"
      aria-expanded={f.open}
      data-state={f.open ? 'open' : 'closed'}
      {...(strip(p).rest as object)}
      onClick={(e) => {
        ;(onClick as ((e: unknown) => void) | undefined)?.(e)
        f.toggle()
      }}
    />
  )
}
export const CollapsibleCardContent = (p: P) => {
  const f = useContext(FoldCtx)
  return <section data-state={f.open ? 'open' : 'closed'} aria-hidden={!f.open} {...(strip(p).rest as object)} />
}
export const Chip = ({ tone, ...p }: P) => <span data-chip data-tone={tone as string} {...(strip(p).rest as object)} />
export const StatusDot = ({ tone, pulse }: P) => <span data-dot={tone as string} data-pulse={pulse ? '' : undefined} />
export const StatusBar = ({ end, children }: P) => (
  <footer>
    {children}
    <span data-slot="end">{end as ReactNode}</span>
  </footer>
)
export const Eyebrow = passthrough('span')
export const Skeleton = passthrough('span')
export const StatusPanel = ({ variant, headline, detail, action, className }: P) => (
  <div role="status" data-variant={variant as string} className={className as string}>
    <b>{headline as ReactNode}</b>
    <span>{detail as ReactNode}</span>
    {action as ReactNode}
  </div>
)
export const EmptyState = ({ title, description, action }: P) => (
  <div data-empty>
    <b>{title as ReactNode}</b>
    <span>{description as ReactNode}</span>
    {action ? (
      <button type="button" onClick={(action as { onClick: () => void }).onClick}>
        {(action as { label: string }).label}
      </button>
    ) : null}
  </div>
)
export const Button = ({ variant, size, asChild, children, ...p }: P) =>
  asChild ? (
    (children as ReactElement)
  ) : (
    <button type="button" data-variant={variant as string} {...(strip(p).rest as object)}>
      {children}
    </button>
  )
export const IconButton = ({ label, variant, ...p }: P) => (
  <button type="button" aria-label={label as string} {...(strip(p).rest as object)} />
)
export const Input = ({ value, onChange, ...p }: P) => (
  <input value={value as string} onChange={(e) => cb(onChange)(e.target.value)} {...(strip(p).rest as object)} />
)
export const ShadcnTextarea = (p: P) => <textarea {...(strip(p).rest as object)} />
export const Select = ({ value, options, groups, onChange, ...p }: P) => {
  const all = [
    ...((options as { value: string; label: string }[] | undefined) ?? []),
    ...((groups as { options: { value: string; label: string }[] }[] | undefined)?.flatMap((g) => g.options) ?? []),
  ]
  return (
    <select
      id={p.id as string}
      value={(value as string) ?? ''}
      onChange={(e) => cb(onChange)(e.target.value)}
      aria-label={p['aria-label'] as string}
    >
      {all.map((o) => (
        <option key={o.value} value={o.value} disabled={(o as { disabled?: boolean }).disabled}>
          {o.label}
        </option>
      ))}
    </select>
  )
}
export const Checkbox = ({ label, checked, onChange }: P) => (
  <label>
    <input type="checkbox" checked={!!checked} onChange={onChange as never} />
    {label as ReactNode}
  </label>
)
export const SegmentedControl = ({ value, options, onChange }: P) => (
  <div role="tablist" data-segmented>
    {(options as { value: string; label: ReactNode }[]).map((o) => (
      <button
        type="button"
        role="tab"
        key={o.value}
        aria-selected={o.value === value}
        onClick={() => cb(onChange)(o.value)}
      >
        {o.label}
      </button>
    ))}
  </div>
)
const TabsCtx = createContext<{ value?: string; set?: (v: string) => void }>({})
export const Tabs = ({ value, onValueChange, children }: P) => (
  <TabsCtx.Provider value={{ value: value as string, set: onValueChange as (v: string) => void }}>
    {children}
  </TabsCtx.Provider>
)
export const TabsList = passthrough('div')
export const TabsTrigger = ({ value, children }: P) => {
  const c = useContext(TabsCtx)
  return (
    <button type="button" role="tab" aria-selected={c.value === value} onClick={() => c.set?.(value as string)}>
      {children}
    </button>
  )
}
export const TabsContent = ({ value, children }: P) => {
  const c = useContext(TabsCtx)
  return c.value === value ? <div role="tabpanel">{children}</div> : null
}
export const List = passthrough('div')
export const ListItem = ({ leading, label, description, trailing, onClick, selected, disabled, ...p }: P) => (
  // biome-ignore lint/a11y/noStaticElementInteractions: a mock row; the real ListItem is a button
  // biome-ignore lint/a11y/useKeyWithClickEvents: same
  // biome-ignore lint/a11y/useAriaPropsSupportedByRole: same
  <div
    role={(p.role as string) ?? 'row'}
    aria-checked={p['aria-checked'] as boolean}
    aria-disabled={(disabled as boolean) || (p['aria-disabled'] as boolean)}
    aria-current={p['aria-current'] as 'page'}
    aria-label={p['aria-label'] as string}
    data-selected={selected || p['data-selected'] !== undefined ? '' : undefined}
    data-step={p['data-step'] as string}
    onClick={disabled ? undefined : (onClick as () => void)}
  >
    {leading as ReactNode}
    <span>{label as ReactNode}</span>
    <span>{description as ReactNode}</span>
    {trailing as ReactNode}
  </div>
)
export const ListGroup = passthrough('div')
export const ListGroupLabel = passthrough('div')
export const TableViewport = passthrough('div')
export const TableFrame = passthrough('div')
export const Table = ({ children }: P) => <table>{children}</table>
export const TableHeader = ({ children }: P) => <thead>{children}</thead>
export const TableBody = ({ children }: P) => <tbody>{children}</tbody>
export const TableRow = ({ children }: P) => <tr>{children}</tr>
export const TableHead = ({ children }: P) => <th>{children}</th>
export const TableCell = ({ children }: P) => <td>{children}</td>
const DialogCtx = createContext(false)
export const Dialog = ({ open, children }: P) => <DialogCtx.Provider value={!!open}>{children}</DialogCtx.Provider>
export const DialogContent = ({ children }: P) => (useContext(DialogCtx) ? <div role="dialog">{children}</div> : null)
export const DialogTitle = ({ children }: P) => <h2>{children}</h2>
export const DialogDescription = ({ children }: P) => <p>{children}</p>
export const DialogClose = ({ children }: P) => children as ReactElement
export const Markdown = ({ children }: P) => <pre data-markdown>{children}</pre>
export const TerminalStream = ({ text, label }: P) => <pre data-terminal={label as string}>{text as string}</pre>
export const MetaRow = ({ items }: P) => (
  <span data-meta>
    {(items as { label: string; value: string }[]).map((i) => (
      <span key={i.label}>{`${i.label} ${i.value}`}</span>
    ))}
  </span>
)
export const SettingsSection = ({ title, children }: P) => (
  <section>
    <h3>{title as ReactNode}</h3>
    {children}
  </section>
)
export const SettingsList = passthrough('div')
export const SettingsRow = ({ label, description }: P) => (
  <div>
    {label as ReactNode} {description as ReactNode}
  </div>
)
export const SettingsField = ({ id, label, meta, renderControl }: P) => (
  <div>
    <label htmlFor={id as string}>{label as ReactNode}</label>
    <span>{meta as ReactNode}</span>
    {(renderControl as (p: { id: string }) => ReactNode)({ id: id as string })}
  </div>
)
export const DropdownMenu = ({ children }: P) => <div data-menu>{children}</div>
export const DropdownMenuTrigger = ({ children }: P) => children as ReactElement
export const DropdownMenuContent = ({ children }: P) => <div role="menu">{children}</div>
export const DropdownMenuSeparator = () => <hr />
const RadioCtx = createContext<(v: string) => void>(() => undefined)
export const DropdownMenuRadioGroup = ({ children, onValueChange }: P) => (
  <RadioCtx.Provider value={onValueChange as (v: string) => void}>
    <div data-group>{children}</div>
  </RadioCtx.Provider>
)
export const DropdownMenuRadioItem = ({ children, value, disabled }: P) => {
  const pick = useContext(RadioCtx)
  return (
    <button
      type="button"
      role="menuitem"
      data-value={value as string}
      disabled={!!disabled}
      onClick={() => pick?.(value as string)}
    >
      {children}
    </button>
  )
}
export const DropdownMenuItem = ({ children, onSelect }: P) => (
  <button type="button" role="menuitem" onClick={onSelect as () => void}>
    {children}
  </button>
)
/**
 * As the console's: a promise keeps it open and busy (confirm disabled), closes it on resolve, shows a rejection.
 * A `cancelLabel` renders the cancel control.
 */
export const ConfirmDialog = ({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  cancelLabel,
  busyLabel,
  onConfirm,
}: P) => {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (open) setError(null)
  }, [open])
  if (!open) return null
  const close = () => (onOpenChange as (o: boolean) => void)(false)
  const confirm = () => {
    const out = (onConfirm as () => unknown)()
    if (!(out instanceof Promise)) return close()
    setBusy(true)
    out.then(
      () => {
        setBusy(false)
        close()
      },
      (e: unknown) => {
        setBusy(false)
        setError(e instanceof Error ? e.message : String(e))
      },
    )
  }
  return (
    <div role="alertdialog" aria-busy={busy || undefined}>
      <b>{title as ReactNode}</b>
      <span>{description as ReactNode}</span>
      {error ? <span role="alert">{error}</span> : null}
      <button type="button" disabled={busy} onClick={confirm}>
        {((busy && busyLabel) || confirmLabel) as ReactNode}
      </button>
      {cancelLabel ? (
        <button type="button" onClick={close}>
          {cancelLabel as ReactNode}
        </button>
      ) : null}
    </div>
  )
}
export const Toolbar = ({ children, end }: P) => (
  <div role="toolbar">
    {children}
    {end as ReactNode}
  </div>
)
export const Breadcrumb = ({ items }: P) => (
  <nav>
    {(items as { label: string; onClick?: () => void }[]).map((i, n) => (
      <span key={i.label}>
        {n ? ' / ' : ''}
        {i.onClick ? (
          <button type="button" onClick={i.onClick}>
            {i.label}
          </button>
        ) : (
          i.label
        )}
      </span>
    ))}
  </nav>
)
export const DropdownMenuGroup = ({ children }: P) => <div data-group>{children}</div>
export const DropdownMenuLabel = ({ children }: P) => <div>{children}</div>
export const DirectoryPicker = ({ value, onChange, emptyLabel }: P) => (
  <input
    aria-label="Folder"
    placeholder={emptyLabel as string | undefined}
    data-directory-picker
    value={(value as string) ?? ''}
    onChange={(e) => cb(onChange)(e.target.value)}
  />
)
