import { Button, Input, SegmentedControl, Select, Skeleton, StatusPanel, Switch } from '@iii-dev/console-ui'
import { type CSSProperties, type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { AlertIcon, ChevronRightIcon, type Control, PlusIcon, storyUrl, UndoIcon, XIcon } from './shared'

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
        <StatusPanel detail={error} headline="The story did not render" icon={<AlertIcon />} variant="alert" />
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

/** The runtime serialises what JSON cannot carry as `{ $kind: … }`. */
function placeholderLabel(value: unknown): string | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const record = value as Record<string, unknown>
  if ('$fn' in record) return `ƒ ${String(record.$fn)}()`
  if ('$element' in record) return `<${String(record.$element)} />`
  if ('$undefined' in record) return 'undefined'
  if ('$date' in record) return `Date ${String(record.$date)}`
  if ('$regexp' in record) return String(record.$regexp)
  if ('$bigint' in record) return `${String(record.$bigint)}n`
  if ('$symbol' in record) return `Symbol(${String(record.$symbol)})`
  if ('$number' in record) return String(record.$number)
  if ('$truncated' in record) return '…'
  if ('$type' in record) return String(record.$type)
  return null
}

const isHexColor = (value: string) => /^#[0-9a-f]{6}$/i.test(value)
const clone = <T,>(value: T): T => (value === undefined ? value : (JSON.parse(JSON.stringify(value)) as T))

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

/** One inspector line: name on the left, editor on the right. */
function Row({ label, hint, depth, overridden, onRevert, caret, trailing, children }: RowProps) {
  return (
    <div
      className="stories-prop"
      data-overridden={overridden ? '' : undefined}
      style={{ '--stories-depth': depth } as CSSProperties}
    >
      <div className="stories-prop__label" title={hint ?? label}>
        {caret}
        <span className="stories-prop__name">{label}</span>
      </div>
      <div className="stories-prop__control">
        {children}
        {trailing}
        {onRevert && overridden ? (
          <button
            aria-label={`revert ${label}`}
            className="stories-prop__action"
            onClick={onRevert}
            title="Revert to the story value"
            type="button"
          >
            <UndoIcon />
          </button>
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
      className="stories-prop__caret"
      onClick={onToggle}
      type="button"
    >
      <ChevronRightIcon />
    </button>
  )
}

function Mono({ value }: { value: unknown }) {
  return <code className="stories-prop__mono">{placeholderLabel(value) ?? JSON.stringify(value) ?? 'undefined'}</code>
}

function BooleanField({ value, onChange, label }: { value: unknown; onChange: (next: unknown) => void; label: string }) {
  return <Switch aria-label={label} checked={Boolean(value)} onChange={(event) => onChange(event.currentTarget.checked)} />
}

function NumberField({ value, onChange, label }: { value: unknown; onChange: (next: unknown) => void; label: string }) {
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
function TextField({ value, onChange, label }: { value: unknown; onChange: (next: unknown) => void; label: string }) {
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

function PickField({
  options,
  value,
  onChange,
  label,
}: {
  options: unknown[]
  value: unknown
  onChange: (next: unknown) => void
  label: string
}) {
  if (Array.isArray(value)) {
    const selected = new Set(value.map(String))
    const toggle = (key: string) => {
      const next = new Set(selected)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      onChange(options.filter((option) => next.has(String(option))))
    }
    return (
      <div aria-label={label} className="stories-prop__picks" role="group">
        {options.map((option) => {
          const key = String(option)
          return (
            <button
              aria-pressed={selected.has(key)}
              className="stories-prop__pick"
              key={key}
              onClick={() => toggle(key)}
              type="button"
            >
              {key}
            </button>
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
function ValueRows(props: ValueProps) {
  const { value, onChange, ...row } = props
  if (placeholderLabel(value) !== null) {
    return (
      <Row {...row}>
        <Mono value={value} />
      </Row>
    )
  }
  if (Array.isArray(value)) return <ListRows {...row} items={value} onChange={onChange} />
  if (value && typeof value === 'object') {
    return <GroupRows {...row} onChange={onChange} record={value as Record<string, unknown>} />
  }
  if (typeof value === 'boolean') {
    return (
      <Row {...row}>
        <BooleanField label={row.label} onChange={onChange} value={value} />
      </Row>
    )
  }
  if (typeof value === 'number') {
    return (
      <Row {...row}>
        <NumberField label={row.label} onChange={onChange} value={value} />
      </Row>
    )
  }
  return (
    <Row {...row}>
      <TextField label={row.label} onChange={onChange} value={value} />
    </Row>
  )
}

function ListRows({ items, onChange, ...row }: Omit<ValueProps, 'value'> & { items: unknown[] }) {
  const [open, setOpen] = useState(true)
  // ponytail: a new item copies the last one; an empty list gets "" — the
  // manifest carries no element type. Read argTypes if that ever matters.
  const add = () => onChange([...items, items.length ? clone(items[items.length - 1]) : ''])
  return (
    <>
      <Row {...row} caret={items.length ? <Caret label={row.label} onToggle={() => setOpen(!open)} open={open} /> : undefined}>
        <span className="stories-prop__summary">
          {items.length} item{items.length === 1 ? '' : 's'}
        </span>
        <button
          aria-label={`add item to ${row.label}`}
          className="stories-prop__action"
          onClick={add}
          title="Add item"
          type="button"
        >
          <PlusIcon />
        </button>
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
                <button
                  aria-label={`remove item ${index} from ${row.label}`}
                  className="stories-prop__action"
                  onClick={() => onChange(items.filter((_, at) => at !== index))}
                  title="Remove item"
                  type="button"
                >
                  <XIcon />
                </button>
              }
              value={item}
            />
          ))
        : null}
    </>
  )
}

function GroupRows({
  record,
  onChange,
  ...row
}: Omit<ValueProps, 'value'> & { record: Record<string, unknown> }) {
  const [open, setOpen] = useState(true)
  const keys = Object.keys(record)
  return (
    <>
      <Row {...row} caret={keys.length ? <Caret label={row.label} onToggle={() => setOpen(!open)} open={open} /> : undefined}>
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
  switch (control.type) {
    case 'boolean':
      return (
        <Row {...row}>
          <BooleanField label={control.name} onChange={onChange} value={value} />
        </Row>
      )
    case 'number':
      return (
        <Row {...row}>
          <NumberField label={control.name} onChange={onChange} value={value} />
        </Row>
      )
    case 'select':
      return (
        <Row {...row}>
          <PickField label={control.name} onChange={onChange} options={control.options ?? []} value={value} />
        </Row>
      )
    case 'text':
      return (
        <Row {...row}>
          <TextField label={control.name} onChange={onChange} value={value} />
        </Row>
      )
    case 'object':
      return <ValueRows {...row} onChange={onChange} value={value} />
    default:
      return (
        <Row {...row}>
          <Mono value={value} />
        </Row>
      )
  }
}

export type InspectorProps = {
  controls: Control[]
  overrides: Record<string, unknown>
  onChange: (next: Record<string, unknown>) => void
}

/** Figma-style property list: dense rows, typed editors, per-prop revert. */
export function Inspector({ controls, overrides, onChange }: InspectorProps) {
  if (controls.length === 0) return <div className="stories-inspector__empty">This state declares no args.</div>
  return (
    <div aria-label="Props" className="stories-inspector" role="group">
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

export function InspectorReset({ overrides, onChange }: Omit<InspectorProps, 'controls'>) {
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
