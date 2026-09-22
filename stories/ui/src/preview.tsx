import {
  Button,
  Checkbox,
  Input,
  SegmentedControl,
  Select,
  Skeleton,
  StatusPanel,
  Switch,
  uiClasses,
} from '@iii-dev/console-ui'
import { ChevronRight, Plus, TriangleAlert, Undo2, X } from 'lucide-react'
import { type CSSProperties, type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { type Control, storyUrl } from './shared'

type FrameMessage = {
  source?: string
  type?: string
  height?: number
  message?: string
}

export type StoryFrameProps = {
  previewUrl: string
  story: string
  args: Record<string, unknown>
  globals: Record<string, unknown>
  title: string
}

/**
 * One story document in a sandboxed iframe. `story` and `globals` travel in
 * the url (a new state remounts the document); `args` overrides are posted
 * to the running runtime, debounced.
 */
export function StoryFrame({ previewUrl, story, args, globals, title }: StoryFrameProps) {
  const frame = useRef<HTMLIFrameElement | null>(null)
  const [height, setHeight] = useState(240)
  const [ready, setReady] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const src = useMemo(() => storyUrl(previewUrl, story, globals), [previewUrl, story, globals])

  useEffect(() => {
    setReady(false)
    setError(null)
  }, [src])

  useEffect(() => {
    const onMessage = (event: MessageEvent<FrameMessage>) => {
      if (!frame.current || event.source !== frame.current.contentWindow) return
      const data = event.data
      if (data?.source !== 'iii-stories') return
      if (data.type === 'stories:rendered') {
        setReady(true)
        setError(null)
        if (typeof data.height === 'number') setHeight(Math.min(4000, Math.max(200, data.height + 2)))
      } else if (data.type === 'stories:error') {
        setReady(true)
        setError(data.message ?? 'The story failed to render.')
      }
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  }, [])

  const argsKey = JSON.stringify(args)
  useEffect(() => {
    if (!ready) return
    const handle = window.setTimeout(() => {
      frame.current?.contentWindow?.postMessage(
        { source: 'iii-stories-host', type: 'stories:render', story, args, globals },
        '*',
      )
    }, 150)
    return () => window.clearTimeout(handle)
  }, [argsKey, ready, story, globals])

  return (
    <div className="stories-frame">
      {!ready ? <Skeleton className="stories-frame__skeleton" /> : null}
      {error ? (
        <StatusPanel
          detail={error}
          headline="The story did not render"
          icon={<TriangleAlert aria-hidden size={16} />}
          variant="alert"
        />
      ) : null}
      <iframe
        className="stories-frame__iframe"
        data-ready={ready ? 'true' : undefined}
        ref={frame}
        sandbox="allow-scripts allow-same-origin"
        src={src}
        style={{ height }}
        title={title}
      />
    </div>
  )
}

export type SideBySideProps = {
  a: StoryFrameProps | null
  b: StoryFrameProps | null
  aLabel: string
  bLabel: string
  narrow: boolean
}

export function SideBySide({ a, b, aLabel, bLabel, narrow }: SideBySideProps) {
  const [side, setSide] = useState<'a' | 'b'>('b')
  const pane = (props: StoryFrameProps | null, label: string) => (
    <section aria-label={label} className="stories-side">
      <header className="stories-side__head">{label}</header>
      {props ? (
        <StoryFrame {...props} />
      ) : (
        <StatusPanel headline="Not in this line" detail="The component does not exist here." variant="info" />
      )}
    </section>
  )
  if (narrow) {
    return (
      <div className="stories-sides stories-sides--single">
        <SegmentedControl
          aria-label="Side"
          onChange={setSide}
          options={[
            { value: 'a', label: aLabel, icon: false },
            { value: 'b', label: bLabel, icon: false },
          ]}
          value={side}
          variant="radio"
        />
        {side === 'a' ? pane(a, aLabel) : pane(b, bLabel)}
      </div>
    )
  }
  return (
    <div className="stories-sides">
      {pane(a, aLabel)}
      {pane(b, bLabel)}
    </div>
  )
}

/* ── props inspector ─────────────────────────────────────────────────── */

/** The runtime serialises what JSON cannot carry as `{ $kind: … }`; first match wins. */
const PLACEHOLDERS: [string, (value: string) => string][] = [
  ['$fn', (v) => `ƒ ${v}()`],
  ['$element', (v) => `<${v} />`],
  ['$undefined', () => 'undefined'],
  ['$date', (v) => `Date ${v}`],
  ['$regexp', (v) => v],
  ['$bigint', (v) => `${v}n`],
  ['$symbol', (v) => `Symbol(${v})`],
  ['$number', (v) => v],
  ['$truncated', () => '…'],
  ['$type', (v) => v],
]

function placeholderLabel(value: unknown): string | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const record = value as Record<string, unknown>
  for (const [key, format] of PLACEHOLDERS) if (key in record) return format(String(record[key]))
  return null
}

const isHexColor = (value: string) => /^#[0-9a-f]{6}$/i.test(value)

type RowProps = {
  label: string
  hint?: string
  depth: number
  overridden?: boolean
  onRevert?: () => void
  caret?: ReactNode
  trailing?: ReactNode
  children?: ReactNode
}

/** A row action in the shared tree-control size (touch-sized under `data-narrow`). */
function RowAction({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <button aria-label={label} className={uiClasses.treeItemAction} onClick={onClick} title={label} type="button">
      {children}
    </button>
  )
}

/** One inspector line: name on the left, editor on the right. */
function Row({ label, hint, depth, overridden, onRevert, caret, trailing, children }: RowProps) {
  return (
    <div
      className="stories-prop"
      data-overridden={overridden ? '' : undefined}
      style={{ '--iii-ui-tree-depth': depth } as CSSProperties}
    >
      <div className="stories-prop__label" title={hint ?? label}>
        {caret}
        <span className="stories-prop__name">{label}</span>
      </div>
      <div className="stories-prop__control">
        {children}
        {trailing}
        {onRevert && overridden ? (
          <RowAction label={`Revert ${label} to the story value`} onClick={onRevert}>
            <Undo2 aria-hidden size={16} />
          </RowAction>
        ) : null}
      </div>
    </div>
  )
}

function Caret({ open, onToggle, label }: { open: boolean; onToggle: () => void; label: string }) {
  return (
    <button
      aria-expanded={open}
      aria-label={`${open ? 'collapse' : 'expand'} ${label}`}
      className={uiClasses.treeItemCaret}
      onClick={onToggle}
      type="button"
    >
      <ChevronRight aria-hidden size={16} />
    </button>
  )
}

function Mono({ value }: { value: unknown }) {
  return <code className="stories-prop__mono">{placeholderLabel(value) ?? JSON.stringify(value) ?? 'undefined'}</code>
}

type FieldProps = { value: unknown; onChange: (next: unknown) => void; label: string }

function BooleanField({ value, onChange, label }: FieldProps) {
  return (
    <Switch aria-label={label} checked={Boolean(value)} onChange={(event) => onChange(event.currentTarget.checked)} />
  )
}

function NumberField({ value, onChange, label }: FieldProps) {
  return (
    <Input
      aria-label={label}
      className="stories-compact stories-compact--number"
      onChange={(next) => onChange(next === '' ? null : Number(next))}
      step="any"
      type="number"
      value={typeof value === 'number' ? String(value) : ''}
    />
  )
}

/** Strings, and `null` (typed into → string; cleared → back to null). */
function TextField({ value, onChange, label }: FieldProps) {
  const nullable = value === null || value === undefined
  const text = nullable ? '' : String(value)
  if (text.includes('\n')) {
    return (
      <textarea
        aria-label={label}
        className="stories-compact stories-compact--area"
        onChange={(event) => onChange(event.currentTarget.value)}
        rows={Math.min(8, text.split('\n').length + 1)}
        spellCheck={false}
        value={text}
      />
    )
  }
  return (
    <>
      {isHexColor(text) ? (
        <input
          aria-label={`${label} colour`}
          className="stories-prop__swatch"
          onChange={(event) => onChange(event.currentTarget.value)}
          type="color"
          value={text}
        />
      ) : null}
      <Input
        aria-label={label}
        className="stories-compact"
        onChange={(next) => onChange(next === '' && nullable ? null : next)}
        placeholder={nullable ? 'null' : undefined}
        value={text}
      />
    </>
  )
}

function PickField({ options, value, onChange, label }: FieldProps & { options: unknown[] }) {
  if (Array.isArray(value)) {
    const selected = new Set(value.map(String))
    return (
      <div aria-label={label} className="stories-prop__picks" role="group">
        {options.map((option) => {
          const key = String(option)
          return (
            <Checkbox
              checked={selected.has(key)}
              key={key}
              label={key}
              onChange={(event) => {
                const checked = event.currentTarget.checked
                onChange(options.filter((o) => (String(o) === key ? checked : selected.has(String(o)))))
              }}
            />
          )
        })}
      </div>
    )
  }
  return (
    <Select
      allowEmpty
      aria-label={label}
      className="stories-compact"
      onChange={(next) => onChange(options.find((option) => String(option) === next) ?? next)}
      onClear={() => onChange(null)}
      options={options.map((option) => ({ value: String(option), label: String(option) }))}
      value={value === null || value === undefined ? undefined : String(value)}
    />
  )
}

type ValueProps = Omit<RowProps, 'caret' | 'children'> & { value: unknown; onChange: (next: unknown) => void }

/** Any JSON shape, by its runtime type: groups and lists expand in place. */
function ValueRows({ value, onChange, ...row }: ValueProps) {
  const opaque = placeholderLabel(value) !== null
  if (!opaque && Array.isArray(value)) return <ListRows {...row} items={value} onChange={onChange} />
  if (!opaque && value && typeof value === 'object') {
    return <GroupRows {...row} onChange={onChange} record={value as Record<string, unknown>} />
  }
  const field = { label: row.label, onChange, value }
  return (
    <Row {...row}>
      {opaque ? (
        <Mono value={value} />
      ) : typeof value === 'boolean' ? (
        <BooleanField {...field} />
      ) : typeof value === 'number' ? (
        <NumberField {...field} />
      ) : (
        <TextField {...field} />
      )}
    </Row>
  )
}

function ListRows({ items, onChange, ...row }: Omit<ValueProps, 'value'> & { items: unknown[] }) {
  const [open, setOpen] = useState(true)
  // ponytail: a new item copies the last one; an empty list gets "" — the
  // manifest carries no element type. Read argTypes if that ever matters.
  const add = () => onChange([...items, items.length ? structuredClone(items[items.length - 1]) : ''])
  return (
    <>
      <Row
        {...row}
        caret={items.length ? <Caret label={row.label} onToggle={() => setOpen(!open)} open={open} /> : undefined}
      >
        <span className="stories-prop__summary">
          {items.length} item{items.length === 1 ? '' : 's'}
        </span>
        <RowAction label={`Add item to ${row.label}`} onClick={add}>
          <Plus aria-hidden size={16} />
        </RowAction>
      </Row>
      {open
        ? items.map((item, index) => (
            <ValueRows
              depth={row.depth + 1}
              // biome-ignore lint/suspicious/noArrayIndexKey: positions are the identity of list items.
              key={index}
              label={String(index)}
              onChange={(next) => onChange(items.map((current, at) => (at === index ? next : current)))}
              trailing={
                <RowAction
                  label={`Remove item ${index} from ${row.label}`}
                  onClick={() => onChange(items.filter((_, at) => at !== index))}
                >
                  <X aria-hidden size={16} />
                </RowAction>
              }
              value={item}
            />
          ))
        : null}
    </>
  )
}

function GroupRows({ record, onChange, ...row }: Omit<ValueProps, 'value'> & { record: Record<string, unknown> }) {
  const [open, setOpen] = useState(true)
  const keys = Object.keys(record)
  return (
    <>
      <Row
        {...row}
        caret={keys.length ? <Caret label={row.label} onToggle={() => setOpen(!open)} open={open} /> : undefined}
      >
        <span className="stories-prop__summary">
          {keys.length ? `${keys.length} field${keys.length === 1 ? '' : 's'}` : '{}'}
        </span>
      </Row>
      {open
        ? keys.map((key) => (
            <ValueRows
              depth={row.depth + 1}
              key={key}
              label={key}
              onChange={(next) => onChange({ ...record, [key]: next })}
              value={record[key]}
            />
          ))
        : null}
    </>
  )
}

function ControlRow({
  control,
  value,
  overridden,
  onChange,
  onRevert,
}: {
  control: Control
  value: unknown
  overridden: boolean
  onChange: (next: unknown) => void
  onRevert: () => void
}) {
  const row = { label: control.name, hint: control.description, depth: 0, overridden, onRevert }
  if (control.type === 'object') return <ValueRows {...row} onChange={onChange} value={value} />
  const field = { label: control.name, onChange, value }
  return (
    <Row {...row}>
      {control.type === 'boolean' ? (
        <BooleanField {...field} />
      ) : control.type === 'number' ? (
        <NumberField {...field} />
      ) : control.type === 'select' ? (
        <PickField {...field} options={control.options ?? []} />
      ) : control.type === 'text' ? (
        <TextField {...field} />
      ) : (
        <Mono value={value} />
      )}
    </Row>
  )
}

export type InspectorProps = {
  controls: Control[]
  overrides: Record<string, unknown>
  onChange: (next: Record<string, unknown>) => void
  narrow?: boolean
}

/** Figma-style property list: dense rows, typed editors, per-prop revert. */
export function Inspector({ controls, overrides, onChange, narrow }: InspectorProps) {
  if (controls.length === 0) return <div className="stories-inspector__empty">This state declares no args.</div>
  return (
    <div
      aria-label="Props"
      className={`${uiClasses.tree} stories-inspector`}
      data-narrow={narrow ? '' : undefined}
      role="group"
    >
      {controls.map((control) => {
        const overridden = control.name in overrides
        return (
          <ControlRow
            control={control}
            key={control.name}
            onChange={(next) => onChange({ ...overrides, [control.name]: next })}
            onRevert={() => {
              const { [control.name]: _dropped, ...rest } = overrides
              onChange(rest)
            }}
            overridden={overridden}
            value={overridden ? overrides[control.name] : control.default}
          />
        )
      })}
    </div>
  )
}

export function InspectorReset({ overrides, onChange }: Pick<InspectorProps, 'overrides' | 'onChange'>) {
  const count = Object.keys(overrides).length
  return (
    <Button disabled={count === 0} onClick={() => onChange({})} size="sm" variant="ghost">
      Reset{count ? ` (${count})` : ''}
    </Button>
  )
}

/** A unified diff, coloured by line prefix. */
export function PatchView({ patch }: { patch: string }) {
  const lines = patch.split('\n')
  return (
    <pre className="stories-patch">
      {lines.map((line, index) => {
        const tone =
          line.startsWith('+++') || line.startsWith('---')
            ? 'meta'
            : line.startsWith('+')
              ? 'add'
              : line.startsWith('-')
                ? 'del'
                : line.startsWith('@@')
                  ? 'hunk'
                  : undefined
        return (
          // biome-ignore lint/suspicious/noArrayIndexKey: patch lines have no identity beyond position.
          <span className="stories-patch__line" data-tone={tone} key={index}>
            {line}
            {'\n'}
          </span>
        )
      })}
    </pre>
  )
}
