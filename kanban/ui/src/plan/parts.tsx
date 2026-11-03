import { ShadcnButton, ShadcnCard } from '@iii-dev/console-ui'
import { ArrowLeft, ChevronRight, ChevronsUpDown, type LucideIcon } from 'lucide-react'
import { forwardRef, type ReactNode } from 'react'
import type { AgentProfile, GlyphTone } from '../shared'
import { AssigneeAvatar } from '../ui/avatar'
import { OptionMenu } from '../ui/option-menu'
import { type PickerOption, SearchPicker } from '../ui/search-picker'
import { humanize } from '../ui/tone'

/** The six identity hues a project can wear. */
export const PROJECT_TONES: GlyphTone[] = ['blue', 'purple', 'teal', 'green', 'amber', 'rose']

const titleCase = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/** A project's identity: a 6px dot in its hue on a 16px slot (a ghost dot without one). Colour lives only here. */
export function FolderGlyph({ color }: { color: GlyphTone | null }) {
  return (
    <span aria-hidden="true" className="kb-plan-glyph">
      <span className="kb-label__dot" data-tone={color ?? undefined} />
    </span>
  )
}

/**
 * A form field's picker trigger: full width, the value (or a quiet placeholder) on the left, the up-down chevron on
 * the right — the shadcn select look, opening a searchable picker.
 */
export const PickerButton = forwardRef<
  HTMLButtonElement,
  { children: ReactNode; placeholder?: boolean; icon?: ReactNode; className?: string; disabled?: boolean }
>(function PickerButton({ children, placeholder, icon, className, ...rest }, ref) {
  return (
    <ShadcnButton
      className={['kb-plan-picker', className].filter(Boolean).join(' ')}
      data-placeholder={placeholder ? 'true' : undefined}
      ref={ref as never}
      type="button"
      variant="outline"
      {...rest}
    >
      {icon}
      <span className="kb-plan-picker__value">{children}</span>
      <ChevronsUpDown className="kb-plan-picker__chevron" size={16} />
    </ShadcnButton>
  )
})

/** The lead of a project: an agent profile, the person at the console, or nobody. */
export function LeadPicker({
  agents,
  value,
  onChange,
  quiet,
  disabled,
}: {
  agents: AgentProfile[]
  value: string | null
  onChange: (lead: string | null) => void
  /** A property row (borderless) instead of a form field. */
  quiet?: boolean
  disabled?: boolean
}) {
  const known = new Map(agents.map((agent) => [agent.id, agent.name]))
  const name = (id: string) => (id === 'user' ? 'User' : (known.get(id) ?? humanize(id)))
  const options: PickerOption[] = [
    { value: '', label: 'No lead', icon: <AssigneeAvatar agents={agents} id={null} /> },
    ...[...agents]
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((agent) => ({
        value: agent.id,
        label: agent.name,
        description: 'Agent',
        icon: <AssigneeAvatar agents={agents} id={agent.id} />,
      })),
    { value: 'user', label: 'User', description: 'Person', icon: <AssigneeAvatar agents={agents} id="user" /> },
    // A lead whose profile is gone still shows as what it is.
    ...(value && value !== 'user' && !known.has(value)
      ? [{ value, label: name(value), icon: <AssigneeAvatar agents={agents} id={value} /> }]
      : []),
  ]
  const trigger = quiet ? (
    <ShadcnButton className="kb-prop" data-empty={value ? undefined : 'true'} data-quiet="true" size="sm" type="button" variant="ghost">
      <AssigneeAvatar agents={agents} id={value} />
      {value ? name(value) : 'Set a lead'}
    </ShadcnButton>
  ) : (
    <PickerButton icon={<AssigneeAvatar agents={agents} id={value} />} placeholder={!value}>
      {value ? name(value) : 'No lead'}
    </PickerButton>
  )
  return (
    <SearchPicker
      disabled={disabled}
      label="Lead"
      onChange={(next) => onChange(next === '' ? null : next)}
      options={options}
      searchPlaceholder="Find a lead…"
      trigger={trigger}
      value={value ?? ''}
    />
  )
}

/** A project's hue as a short menu of folders; `allowNone` adds "No color". */
export function TonePicker({
  value,
  onChange,
  allowNone,
  quiet,
}: {
  value: GlyphTone | null
  onChange: (tone: GlyphTone | null) => void
  allowNone?: boolean
  quiet?: boolean
}) {
  const options = [
    ...(allowNone ? [{ value: '', label: 'No color', icon: <FolderGlyph color={null} /> }] : []),
    ...PROJECT_TONES.map((tone) => ({ value: tone, label: titleCase(tone), icon: <FolderGlyph color={tone} /> })),
  ]
  const trigger = quiet ? (
    <ShadcnButton className="kb-prop" data-quiet="true" size="sm" type="button" variant="ghost">
      <FolderGlyph color={value} />
      {value ? titleCase(value) : 'No color'}
    </ShadcnButton>
  ) : (
    <PickerButton icon={<FolderGlyph color={value} />}>{value ? titleCase(value) : 'No color'}</PickerButton>
  )
  return (
    <OptionMenu
      label="Color"
      onChange={(next) => onChange(next === '' ? null : (next as GlyphTone))}
      options={options}
      trigger={trigger}
      value={value ?? ''}
    />
  )
}

/** A titled card section of a detail page: heading, an optional trailing control, then the body. */
export function Section({
  title,
  count,
  action,
  children,
  flush,
}: {
  title: string
  count?: number
  action?: ReactNode
  children: ReactNode
  /** The body runs to the card's edges (a list of rows). */
  flush?: boolean
}) {
  return (
    <ShadcnCard aria-label={title} className="kb-plan-section" role="region" size="sm">
      <header className="kb-plan-section__head">
        <h3 className="kb-plan-section__title">{title}</h3>
        {count !== undefined ? <span className="kb-plan-section__count">{count}</span> : null}
        <span className="kb-plan-spacer" />
        {action}
      </header>
      <div className="kb-plan-section__body" data-flush={flush ? 'true' : undefined}>
        {children}
      </div>
    </ShadcnCard>
  )
}

/** One property row: a quiet label column, then the value. */
export function Prop({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="kb-plan-prop">
      <span className="kb-plan-prop__label">{label}</span>
      <div className="kb-plan-prop__value">{children}</div>
    </div>
  )
}

/** A neutral note with an icon and an optional action — informative, not an error. */
export function Callout({
  icon: Icon,
  title,
  detail,
  action,
}: {
  icon: LucideIcon
  title: ReactNode
  detail?: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="kb-plan-callout" role="status">
      <Icon size={16} />
      <div className="kb-plan-callout__text">
        <p className="kb-plan-callout__title">{title}</p>
        {detail ? <p className="kb-plan-callout__detail">{detail}</p> : null}
      </div>
      {action}
    </div>
  )
}

/** The way back from one record: `← Releases › v0.25`. */
export function Crumbs({ root, name, onBack }: { root: string; name: string; onBack: () => void }) {
  return (
    <nav aria-label={`${root} navigation`} className="kb-plan-crumbs">
      <ShadcnButton aria-label={`Back to the ${root.toLowerCase()}`} onClick={onBack} size="sm" type="button" variant="ghost">
        <ArrowLeft size={16} />
        {root}
      </ShadcnButton>
      <ChevronRight aria-hidden="true" className="kb-plan-crumbs__sep" size={16} />
      <span aria-current="page" className="kb-plan-crumbs__here">
        {name}
      </span>
    </nav>
  )
}
