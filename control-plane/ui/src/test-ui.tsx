import type { Host } from '@iii-dev/console-ui'
import { createContext, useContext, type ReactNode } from 'react'
import { beforeEach, afterEach, vi } from 'vitest'

type Props = Record<string, any>
const Box = ({ children, className, ...props }: Props) => (
  <div className={className} role={props.role} aria-label={props['aria-label']} aria-busy={props['aria-busy']}>
    {children}
  </div>
)
const TabsContext = createContext<((value: string) => void) | undefined>(undefined)
export const mockUi = {
  uiClasses: { spin: 'iii-ui-spin', pulse: 'iii-ui-pulse', eyebrow: 'iii-ui-eyebrow' },
  Button: ({ children, variant, size: _size, ...props }: Props) => (
    <button type="button" data-variant={variant} {...props}>
      {children}
    </button>
  ),
  IconButton: ({ children, label, ...props }: Props) => (
    <button type="button" aria-label={label} {...props}>
      {children}
    </button>
  ),
  PageShell: Box,
  PageBody: Box,
  PageMain: Box,
  PageSidebar: Box,
  PageHeader: ({ title, children, actions }: Props) => (
    <header>
      <h1>{title}</h1>
      {children}
      {actions}
    </header>
  ),
  Card: Box,
  CardBody: Box,
  SettingsList: Box,
  SettingsField: ({ label, field, renderControl }: Props) => (
    <label>{label}{renderControl({ 'data-field': field })}</label>
  ),
  PanelHeader: Box,
  List: Box,
  ListGroup: Box,
  ListGroupLabel: Box,
  ListItem: ({ label, description, trailing, selected, ...props }: Props) => (
    <button type="button" aria-selected={selected} {...props}>
      {label}
      {description}
      {trailing}
    </button>
  ),
  StatusBar: ({ children, end }: Props) => (
    <footer>
      {children}
      {end}
    </footer>
  ),
  StatusDot: ({ tone }: Props) => <i className={`bg-${tone}`} />,
  StatusPanel: ({ headline, detail, action, role, className }: Props) => (
    <div role={role ?? 'status'} className={className}>
      {headline}
      {detail}
      {action}
    </div>
  ),
  EmptyState: ({ title, description, action }: Props) => (
    <div>
      <h2>{title}</h2>
      <p>{description}</p>
      {action ? (
        <button type="button" onClick={action.onClick}>
          {action.label}
        </button>
      ) : null}
    </div>
  ),
  Skeleton: Box,
  Eyebrow: ({ as: Tag = 'span', children, className }: Props) => <Tag className={className}>{children}</Tag>,
  Input: ({ onChange, ...props }: Props) => <input {...props} onChange={(event) => onChange(event.target.value)} />,
  SearchField: ({ onChange, ...props }: Props) => (
    <input {...props} onChange={(event) => onChange(event.target.value)} />
  ),
  ModelPicker: ({ value, options, onChange, automatic, loading, thinkingLevel, onThinkingLevelChange, showReasoningEffort }: Props) => (
    <>
      <select
        data-model-picker
        aria-busy={loading}
        value={value ?? ''}
        onChange={(event) => (event.target.value ? onChange(event.target.value) : automatic?.onSelect())}
      >
        {automatic ? <option value="">{automatic.label}</option> : null}
        {options.map((option: { id: string; label: string }) => (
          <option key={option.id} value={option.id}>
            {option.label}
          </option>
        ))}
      </select>
      {showReasoningEffort ? (
        <select data-reasoning-picker value={thinkingLevel} onChange={(event) => onThinkingLevelChange(event.target.value)}>
          <option value="default">default</option>
          {(options.find((option: { id: string }) => option.id === value)?.reasoningEfforts ?? []).map(
            (row: { effort: string }) => (
              <option key={row.effort} value={row.effort}>
                {row.effort}
              </option>
            ),
          )}
        </select>
      ) : null}
    </>
  ),
  Select: ({ options, value, onChange, ...props }: Props) => (
    <select value={value} onChange={(event) => onChange(event.target.value)} {...props}>
      {options.map((option: Props) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  ),
  Tabs: ({ children, onValueChange }: Props) => (
    <TabsContext.Provider value={onValueChange}>{children}</TabsContext.Provider>
  ),
  TabsList: Box,
  TabsTrigger: ({ children, value }: Props) => {
    const onChange = useContext(TabsContext)
    return (
      <button type="button" onClick={() => onChange?.(value)}>
        {children}
      </button>
    )
  },
  SegmentedControl: ({ options, onChange }: Props) => (
    <div>
      {options.map((option: Props) => (
        <button type="button" key={option.value} onClick={() => onChange(option.value)}>
          {option.label}
        </button>
      ))}
    </div>
  ),
  BottomSheet: ({ open, children }: Props) => (open ? children : null),
  BottomSheetContent: Box,
}
export const host = {
  iii: { call: vi.fn() },
  panels: { open: vi.fn(), openScreen: vi.fn() },
  chat: { openConversation: vi.fn() },
} as unknown as Host
export const TooltipProvider = ({ children }: { children: ReactNode }) => children

beforeEach(() => {
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
  )
})
afterEach(() => vi.unstubAllGlobals())
