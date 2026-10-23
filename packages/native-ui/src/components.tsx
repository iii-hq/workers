/**
 * One component per catalog type (design §6, §10.3). Each creates its node,
 * keeps every prop in sync with a render effect (enums by name, `on*`
 * handlers into the `events` mask), and inserts its slots: `children` is
 * slot 0, the other slots are props named after them (`actions`, `top`,
 * `trailing`, …).
 */

import { createRenderEffect, type JSX, untrack } from 'solid-js'
import { EVENTS } from './generated/events'
import type {
  BadgeProps,
  BoxProps,
  ButtonProps,
  CardProps,
  ChipProps,
  ColumnProps,
  DividerProps,
  EmptyStateProps,
  IconButtonProps,
  IconProps,
  ImageProps,
  ListDetailProps,
  ListRowProps,
  PageProps,
  ProgressProps,
  RemoteScreenProps,
  RowProps,
  ScrollProps,
  SegmentedProps,
  SheetProps,
  SpacerProps,
  SpinnerProps,
  StatusDotProps,
  StatusPanelProps,
  SwitchProps,
  TextFieldProps,
  TextProps,
} from './generated/props'
import { insert } from './renderer'
import {
  createNode,
  ElementNode,
  isBinding,
  recordProp,
  resendProp,
  sameValue,
  setHandler,
  setNamedProp,
  setProp,
  slotContainer,
  warnOnce,
} from './runtime/tree'

type AnyProps = Record<string, unknown>

/** Keeps every plain prop and handler of `props` on `node`; `skip` names the ones the component owns. */
export function applyProps(node: ElementNode, props: AnyProps, skip?: ReadonlySet<string>): void {
  for (const key of Object.keys(props)) {
    if (key === 'children' || key === 'ref' || key in node.spec.slots || skip?.has(key)) continue
    createRenderEffect(() => setNamedProp(node, key, props[key]))
  }
  const ref = props.ref
  if (typeof ref === 'function') untrack(() => ref(node))
}

/** Inserts the slot props of `props` (slot 0 takes `children` and, when it has another name, that prop too). */
export function applySlots(node: ElementNode, props: AnyProps): void {
  let hasChildrenSlot = false
  for (const [name, [slot, , template]] of Object.entries(node.spec.slots)) {
    if (template) continue
    if (slot === 0) {
      hasChildrenSlot = true
      const named = name !== 'children' && name in props
      const kids = 'children' in props
      if (named && kids) insert(node, () => [props[name], props.children] as JSX.Element)
      else if (named) insert(node, () => props[name] as JSX.Element)
      else if (kids) insert(node, () => props.children as JSX.Element)
    } else if (name in props) {
      insert(slotContainer(node, slot), () => props[name] as JSX.Element)
    }
  }
  if (!hasChildrenSlot && 'children' in props && props.children != null) {
    warnOnce(
      `${node.typeName} takes no children (its slots are props: ${Object.keys(node.spec.slots).join(', ') || 'none'})`,
    )
  }
}

function make<P>(typeName: string): (props: P) => JSX.Element {
  const component = (props: P) => {
    const node = createNode(typeName)
    applyProps(node, props as AnyProps)
    applySlots(node, props as AnyProps)
    return node as unknown as JSX.Element
  }
  Object.defineProperty(component, 'name', { value: typeName })
  return component
}

/** The text of JSX children: strings and numbers joined; one template binding passes through. */
function childText(value: unknown, typeName: string): unknown {
  while (typeof value === 'function') value = value()
  if (value === undefined) return undefined
  if (value === null || typeof value === 'boolean') return ''
  if (typeof value === 'string' || typeof value === 'number') return String(value)
  if (isBinding(value)) return value
  if (Array.isArray(value)) {
    const parts = value.map((v) => childText(v, typeName)).filter((v) => v !== undefined && v !== '')
    if (parts.length === 1) return parts[0]
    if (parts.some(isBinding)) {
      warnOnce(`${typeName}: a template binding cannot be joined with other text; the host does not format`)
      return parts.filter((p) => !isBinding(p)).join('')
    }
    return parts.join('')
  }
  if (value instanceof ElementNode) {
    warnOnce(`${typeName} holds text only; ${value.typeName} inside it is ignored`)
    return ''
  }
  return String(value)
}

const TEXT_PROP = new Set(['text'])

function makeText<P>(typeName: string): (props: P) => JSX.Element {
  const component = (props: P) => {
    const p = props as AnyProps
    const node = createNode(typeName)
    applyProps(node, p, TEXT_PROP)
    if ('text' in p || 'children' in p) {
      createRenderEffect(() =>
        setNamedProp(node, 'text', p.text !== undefined ? p.text : childText(p.children, typeName)),
      )
    }
    return node as unknown as JSX.Element
  }
  Object.defineProperty(component, 'name', { value: typeName })
  return component
}

/**
 * Re-sends the value a Switch or Segmented shows when the app did not take
 * the person's change (the host shows it optimistically until the next set).
 */
function resyncAfter(node: ElementNode, event: number, prop: number, encode: (arg: unknown) => unknown): void {
  node.afterEvent = (ev, args) => {
    if (ev !== event) return
    const current = node.props.has(prop) ? node.props.get(prop) : defaultOf(node, prop)
    if (!sameValue(current, encode(args[0]))) resendProp(node, prop)
  }
}

function defaultOf(node: ElementNode, tag: number): unknown {
  for (const spec of Object.values(node.spec.props)) if (spec[0] === tag) return spec[3]
  return undefined
}

// ── Layout ──────────────────────────────────────────────────────────────────

export const Column = make<ColumnProps>('Column')
export const Row = make<RowProps>('Row')
export const Box = make<BoxProps>('Box')
export const Scroll = make<ScrollProps>('Scroll')
export const Spacer = make<SpacerProps>('Spacer')
export const Divider = make<DividerProps>('Divider')

// ── Content ─────────────────────────────────────────────────────────────────

export const Text = makeText<TextProps>('Text')
export const Icon = make<IconProps>('Icon')
export const Badge = makeText<BadgeProps>('Badge')
export const StatusDot = make<StatusDotProps>('StatusDot')
export const Spinner = make<SpinnerProps>('Spinner')
export const Progress = make<ProgressProps>('Progress')
export const EmptyState = make<EmptyStateProps>('EmptyState')
export const Image = make<ImageProps>('Image')
export const StatusPanel = make<StatusPanelProps>('StatusPanel')

// ── Inputs ──────────────────────────────────────────────────────────────────

export const Button = makeText<ButtonProps>('Button')
export const IconButton = make<IconButtonProps>('IconButton')
export const Chip = makeText<ChipProps>('Chip')

const TEXT_FIELD_OWN = new Set(['value', 'onChange'])
const VALUE = 16

/**
 * An uncontrolled text field (design §5.6): the text belongs to the host.
 * `onChange(text)` gets each edit of the person; `value` is only a write,
 * sent with the count of edits this field has seen, and the host drops it
 * when the person typed again meanwhile. Keep `value` in a signal that
 * `onChange` updates; a write equal to the host's text is not sent (it
 * would move the cursor to the end).
 */
export function TextField(props: TextFieldProps): JSX.Element {
  const p = props as AnyProps
  const node = createNode('TextField')
  applyProps(node, p, TEXT_FIELD_OWN)
  let count = 0
  let hostText = ''
  let wanted: string | undefined
  if ('value' in p || 'onChange' in p) {
    node.onText = (c, text) => {
      count = c
      hostText = text
      recordProp(node, VALUE, [text, c])
      ;(p.onChange as ((text: string) => void) | undefined)?.(text)
    }
    // Subscribing to change turns the text entries on.
    setHandler(node, EVENTS.change, () => {})
  }
  node.onRevive = () => {
    // A new host node starts empty with no edits.
    count = 0
    hostText = ''
    hostText = wanted ?? ''
    if (wanted !== undefined) recordProp(node, VALUE, [wanted, 0])
  }
  if ('value' in p) {
    createRenderEffect(() => {
      const v = p.value
      if (v === undefined) return
      const text = v === null ? '' : String(v)
      wanted = text
      if (text === hostText) {
        recordProp(node, VALUE, [text, count])
        return
      }
      hostText = text
      setProp(node, VALUE, [text, count])
    })
  }
  return node as unknown as JSX.Element
}

export function Switch(props: SwitchProps): JSX.Element {
  const node = createNode('Switch')
  applyProps(node, props as AnyProps)
  resyncAfter(node, EVENTS.change, 16, (v) => v === true)
  return node as unknown as JSX.Element
}

export function Segmented(props: SegmentedProps): JSX.Element {
  const node = createNode('Segmented')
  applyProps(node, props as AnyProps)
  resyncAfter(node, EVENTS.select, 17, (v) => (typeof v === 'number' ? Math.trunc(v) : 0))
  return node as unknown as JSX.Element
}

// ── Collections (List lives in list.ts) ─────────────────────────────────────

export const ListRow = make<ListRowProps>('ListRow')
export const Card = make<CardProps>('Card')

// ── Page chrome, overlays, high level ───────────────────────────────────────

export const Page = make<PageProps>('Page')
export const ListDetail = make<ListDetailProps>('ListDetail')
export const Sheet = make<SheetProps>('Sheet')
export const RemoteScreen = make<RemoteScreenProps>('RemoteScreen')
