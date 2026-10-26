/**
 * The SDK's mirror of the host's node tree (design §5.2, §5.3): parents,
 * slots, children, props, handlers and ids.
 *
 * A node reaches the host lazily. Creating nodes, setting their props and
 * inserting them into each other is only recorded until the subtree is
 * attached to a node the host already has (or becomes a page root); then
 * the whole subtree goes out in one canonical walk: `create`, the type's
 * props by tag, the common props by tag, then each slot's children (each one
 * walked the same way, then `insert`ed). Ops on nodes the host has go out
 * as they happen. At the end of every batch, a node the host has that ended
 * detached (not a page root) gets `destroy`, after every other op; the
 * host frees it with everything attached under it. If the app attaches such
 * a node again later, it goes out again under a new id.
 */

import {
  COMMON_PROPS,
  ENUMS,
  type EnumName,
  OP,
  type PropSpec,
  SINCE,
  TYPES,
  type TypeSpec,
} from '../generated/catalog'
import type { NodeRef } from '../types'
import { dropLastEntry, ids, lastEntry, onFlush, push, schedule } from './wire'

export const ELEMENT = 1
export const TEXT = 2
export const SLOT = 3

/** The `events` common prop: a bitmask of `1 << event tag`. */
const EVENTS_PROP = 2

/** Text outside a text-taking component: kept for Solid's bookkeeping, never sent. */
export interface TextNode {
  readonly kind: typeof TEXT
  value: string
  parent: Container | null
}

/** The children of one slot (1 and up) of an element; invisible to the host. */
export interface SlotNode {
  readonly kind: typeof SLOT
  readonly owner: ElementNode
  readonly slot: number
  readonly children: Child[]
  readonly parent: null
}

export type Container = ElementNode | SlotNode
export type Child = ElementNode | TextNode
export type AnyNode = ElementNode | TextNode | SlotNode

/** What a List node needs from its data source. */
export interface ListData {
  /** The items now, untracked. */
  snapshot(): readonly unknown[]
  get(key: unknown): unknown
  detach(node: ElementNode): void
}

export class ElementNode implements NodeRef {
  readonly kind = ELEMENT
  /** The wire id; a node the host destroyed gets a new one when it goes out again. */
  id: number
  readonly type: number
  readonly spec: TypeSpec
  parent: Container | null = null
  /** Slot 0. */
  readonly children: Child[] = []
  /** Slots 1 and up, by index. */
  readonly slots: (SlotNode | undefined)[] = []
  /** Prop tag → its wire value. */
  readonly props = new Map<number, unknown>()
  handlers: Map<number, (...args: any[]) => void> | null = null
  /** The host has this node. */
  live = false
  /** The host destroyed this node; it needs a new id to go out again. */
  retired = false
  /** The page instance this node is the root of, or 0. */
  page = 0

  /** List: its data. */
  list: ListData | null = null
  /** TextField: a `text` entry from the host. */
  onText: ((count: number, text: string) => void) | null = null
  /** After a handler ran for an event of this node (Switch and Segmented re-sync their value). */
  afterEvent: ((event: number, args: unknown[]) => void) | null = null
  /** Right after `create` and the props went out: a List sends its items. */
  onLive: (() => void) | null = null
  /** Before the node goes out again under a new id (TextField restarts its edit count). */
  onRevive: (() => void) | null = null

  constructor(
    readonly typeName: string,
    spec: TypeSpec,
  ) {
    this.id = ids.node++
    this.type = spec.tag
    this.spec = spec
  }
}

const liveById = new Map<number, ElementNode>()
const pageRoots = new Map<number, ElementNode>()
/** Nodes the host has that may have ended detached in this batch. */
const touched = new Set<ElementNode>()

/** A node the host has, by wire id. */
export function nodeById(id: unknown): ElementNode | undefined {
  return liveById.get(id as number)
}

/** The catalogue version the app declared in `defineApp` (what its `ready` says). */
export const appCatalog = { version: 1 }

/**
 * Warns once when `name` (a type, `Type.prop` or `Type.onEvent`) is newer
 * than the declared catalogue: an iii app of that older catalogue would
 * still run the bundle, and draw a placeholder or ignore the prop.
 */
export function checkSince(name: string): void {
  const since = SINCE[name]
  if (since !== undefined && since > appCatalog.version) {
    warnOnce(
      `${name} is catalogue ${since}, and this app declares ${appCatalog.version}: defineApp({ catalog: ${since}, … }), and NativeUi::catalog(${since}) in the worker`,
    )
  }
}

export function createNode(typeName: string): ElementNode {
  const spec = (TYPES as unknown as Record<string, TypeSpec>)[typeName]
  if (!spec) {
    throw new Error(
      `<${typeName}> is not a native-ui component: use the components of @iii-dev/native-ui (Column, Text, Button, …)`,
    )
  }
  checkSince(typeName)
  return new ElementNode(typeName, spec)
}

export function createText(value: string): TextNode {
  return { kind: TEXT, value, parent: null }
}

/** The container of `slot` (0 is the element itself). */
export function slotContainer(node: ElementNode, slot: number): Container {
  if (slot === 0) return node
  let s = node.slots[slot]
  if (!s) {
    s = { kind: SLOT, owner: node, slot, children: [], parent: null }
    node.slots[slot] = s
  }
  return s
}

function ownerOf(container: Container): ElementNode {
  return container.kind === SLOT ? container.owner : container
}

function slotOf(container: Container): number {
  return container.kind === SLOT ? container.slot : 0
}

/** The index the host sees: element children only. */
function wireIndex(list: Child[], child: Child): number {
  let n = 0
  for (const c of list) {
    if (c === child) return n
    if (c.kind === ELEMENT) n++
  }
  return n
}

let warnedText = false

export function insertChild(container: Container, child: Child, anchor?: Child | null): void {
  if (child.parent === container) {
    moveChild(container, child, anchor)
    return
  }
  if (child.parent) removeChild(child.parent, child)
  const list = container.children
  const at = anchor ? list.indexOf(anchor) : -1
  if (at < 0) list.push(child)
  else list.splice(at, 0, child)
  child.parent = container
  if (child.kind !== ELEMENT) {
    if (!warnedText && child.value.trim()) {
      warnedText = true
      console.warn(
        `native-ui: text outside a Text, Button, Badge or Chip is not shown ("${child.value.slice(0, 40)}" in ${ownerOf(container).typeName})`,
      )
    }
    return
  }
  const owner = ownerOf(container)
  if (owner.live) {
    const index = wireIndex(list, child)
    if (
      lastRemoval &&
      lastEntry() === lastRemoval.entry &&
      lastRemoval.node === child &&
      lastRemoval.from === container
    ) {
      // Solid moves some nodes with a remove and an insert into the same
      // slot; the host gets the move the SDK promises (design §5.3).
      dropLastEntry()
      if (lastRemoval.index !== index) push([OP.move, owner.id, slotOf(container), lastRemoval.index, index])
      lastRemoval = null
      return
    }
    materialize(child)
    push([OP.insert, owner.id, slotOf(container), index, child.id])
  } else if (child.live) {
    // The host has the child detached; it goes back in with its new parent,
    // or gets destroyed at the end of the batch.
    touched.add(child)
    schedule()
  }
}

function moveChild(container: Container, child: Child, anchor?: Child | null): void {
  const list = container.children
  const from = wireIndex(list, child)
  list.splice(list.indexOf(child), 1)
  let at = anchor && anchor !== child ? list.indexOf(anchor) : -1
  if (at < 0) at = list.length
  list.splice(at, 0, child)
  if (child.kind !== ELEMENT) return
  const owner = ownerOf(container)
  const to = wireIndex(list, child)
  if (owner.live && from !== to) push([OP.move, owner.id, slotOf(container), from, to])
}

export function removeChild(container: Container, child: Child): void {
  const list = container.children
  const i = list.indexOf(child)
  if (i < 0) return
  const index = wireIndex(list, child)
  list.splice(i, 1)
  child.parent = null
  if (child.kind !== ELEMENT) return
  const owner = ownerOf(container)
  if (owner.live && child.live) {
    const entry = [OP.remove, owner.id, slotOf(container), index]
    push(entry)
    lastRemoval = { entry, node: child, from: container, index }
    touched.add(child)
  }
}

/** The removal just sent, while it is the last entry of the batch. */
let lastRemoval: { entry: unknown[]; node: ElementNode; from: Container; index: number } | null = null

/** The type's props by tag, then the common props by tag (design §5.7's order). */
function wireOrder(a: number, b: number): number {
  const ta = a >= 16 ? 0 : 1
  const tb = b >= 16 ? 0 : 1
  return ta - tb || a - b
}

const defaultsByType = new Map<string, Map<number, unknown>>()

function defaultsOf(node: ElementNode): Map<number, unknown> {
  let d = defaultsByType.get(node.typeName)
  if (!d) {
    d = new Map()
    for (const spec of Object.values(COMMON_PROPS)) d.set(spec[0], spec[3])
    for (const spec of Object.values(node.spec.props)) d.set(spec[0], spec[3])
    defaultsByType.set(node.typeName, d)
  }
  return d
}

const specsByType = new Map<string, Map<number, PropSpec>>()

/** The spec of prop `tag` of `node` (its type's, then the common ones). */
export function propSpecOf(node: ElementNode, tag: number): PropSpec | undefined {
  let specs = specsByType.get(node.typeName)
  if (!specs) {
    specs = new Map()
    for (const spec of Object.values(COMMON_PROPS)) specs.set(spec[0], spec)
    for (const spec of Object.values(node.spec.props)) specs.set(spec[0], spec)
    specsByType.set(node.typeName, specs)
  }
  return specs.get(tag)
}

/** Same wire value (props hold primitives, small arrays and binding objects). */
export function sameValue(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') return false
  return JSON.stringify(a) === JSON.stringify(b)
}

/** Sends `node` and everything under it to the host (no-op when the host has it). */
function materialize(node: ElementNode): void {
  if (node.live) return
  if (node.retired) {
    node.id = ids.node++
    node.retired = false
    node.onRevive?.()
  }
  node.live = true
  liveById.set(node.id, node)
  push([OP.create, node.id, node.type])
  const defaults = defaultsOf(node)
  for (const tag of [...node.props.keys()].sort(wireOrder)) {
    const value = node.props.get(tag)
    if (!sameValue(value, defaults.get(tag))) push([OP.set, node.id, tag, value])
  }
  materializeSlot(node, node.children, 0)
  for (let slot = 1; slot < node.slots.length; slot++) {
    const s = node.slots[slot]
    if (s) materializeSlot(node, s.children, slot)
  }
  node.onLive?.()
}

function materializeSlot(node: ElementNode, list: Child[], slot: number): void {
  let index = 0
  for (const child of list) {
    if (child.kind !== ELEMENT) continue
    materialize(child)
    push([OP.insert, node.id, slot, index++, child.id])
  }
}

/** The host freed `node` and everything attached under it. */
function retire(node: ElementNode): void {
  if (!node.live) return
  node.live = false
  node.retired = true
  liveById.delete(node.id)
  for (const c of node.children) if (c.kind === ELEMENT) retire(c)
  for (const s of node.slots) if (s) for (const c of s.children) if (c.kind === ELEMENT) retire(c)
}

function attachedOnHost(node: ElementNode): boolean {
  if (node.page) return true
  return node.parent !== null && ownerOf(node.parent).live
}

onFlush(() => {
  if (!touched.size) return
  const nodes = [...touched]
  touched.clear()
  for (const node of nodes) {
    if (!node.live || attachedOnHost(node)) continue
    push([OP.destroy, node.id])
    retire(node)
  }
})

/** Sets (or, with `undefined`, resets) one prop by tag; the host hears of it when it has the node. */
export function setProp(node: ElementNode, tag: number, value: unknown): void {
  const has = node.props.has(tag)
  if (value === undefined) {
    if (!has) return
    node.props.delete(tag)
    if (node.live) push([OP.set, node.id, tag, null])
    return
  }
  if (has && sameValue(node.props.get(tag), value)) return
  node.props.set(tag, value)
  if (node.live) push([OP.set, node.id, tag, value])
}

/** Records a prop's value without telling the host (it already shows it: a TextField's own text). */
export function recordProp(node: ElementNode, tag: number, value: unknown): void {
  node.props.set(tag, value)
}

/** Sends the prop's current value again (the host showed something else optimistically). */
export function resendProp(node: ElementNode, tag: number): void {
  if (node.live) push([OP.set, node.id, tag, node.props.has(tag) ? node.props.get(tag) : null])
}

export function setHandler(node: ElementNode, event: number, handler: ((...args: any[]) => void) | undefined): void {
  if (handler) (node.handlers ??= new Map()).set(event, handler)
  else node.handlers?.delete(event)
  let mask = 0
  if (node.handlers) for (const tag of node.handlers.keys()) mask |= 1 << tag
  setProp(node, EVENTS_PROP, mask === 0 ? undefined : mask)
}

const warned = new Set<string>()

export function warnOnce(message: string): void {
  if (warned.has(message)) return
  warned.add(message)
  console.warn(`native-ui: ${message}`)
}

/** Whether `value` is a List template binding (`{"$item": "path"}`). */
export function isBinding(value: unknown): value is { $item: string } {
  return typeof value === 'object' && value !== null && typeof (value as { $item?: unknown }).$item === 'string'
}

/** The wire value of a prop (`undefined` = the default), checked against its catalog type. */
export function encodeProp(spec: PropSpec, value: unknown, label: string): unknown {
  if (value === undefined || value === null) return undefined
  if (isBinding(value)) return { $item: value.$item }
  const [, type, enumName] = spec
  switch (type) {
    case 'bool':
      return Boolean(value)
    case 'int': {
      const n = Math.trunc(Number(value))
      return Number.isFinite(n) ? n : undefined
    }
    case 'float':
    case 'dp': {
      const n = Number(value)
      return Number.isFinite(n) ? n : undefined
    }
    case 'string':
    case 'icon':
      return String(value)
    case 'strings':
      return Array.isArray(value) ? value.map(String) : undefined
    case 'padding':
      if (typeof value === 'number') return Number.isFinite(value) ? value : undefined
      if (Array.isArray(value) && (value.length === 2 || value.length === 4)) return value.map((v) => Number(v) || 0)
      warnOnce(`${label}: padding is a number, [vertical, horizontal] or [top, end, bottom, start]`)
      return undefined
    case 'enum': {
      const values = ENUMS[enumName as EnumName] as readonly string[]
      const index = typeof value === 'number' ? value : values.indexOf(value as string)
      if (Number.isInteger(index) && index >= 0 && index < values.length) return index
      warnOnce(`${label}: ${JSON.stringify(value)} is not one of ${values.join(', ')}`)
      return undefined
    }
    default:
      return value
  }
}

/** Sets a prop or an event handler by its JSX name (`gap`, `onPress`). */
export function setNamedProp(node: ElementNode, name: string, value: unknown): void {
  if (value !== undefined && value !== null) checkSince(`${node.typeName}.${name}`)
  const spec = node.spec.props[name] ?? (name === 'events' ? undefined : COMMON_PROPS[name])
  if (spec) {
    setProp(node, spec[0], encodeProp(spec, value, `${node.typeName}.${name}`))
    return
  }
  const event = node.spec.events[name]
  if (event !== undefined) {
    setHandler(node, event, typeof value === 'function' ? (value as (...args: any[]) => void) : undefined)
    return
  }
  if (/^on[A-Z]/.test(name)) warnOnce(`${node.typeName} has no ${name} event`)
  else warnOnce(`${node.typeName} has no prop ${name}`)
}

/** Makes `node` the root of page instance `page` (null clears it), design §5.3 `root`. */
export function setPageRoot(page: number, node: ElementNode | null): void {
  const current = pageRoots.get(page) ?? null
  if (current === node) return
  if (current) dropRoot(current)
  if (!node) {
    pageRoots.delete(page)
    push([OP.root, page, 0])
    return
  }
  if (node.parent) removeChild(node.parent, node)
  node.page = page
  pageRoots.set(page, node)
  materialize(node)
  push([OP.root, page, node.id])
}

/** Whether page instance `page` has a root node now. */
export function hasPageRoot(page: number): boolean {
  return pageRoots.has(page)
}

/** The page instance left the deck: its root goes at the end of the batch, no `root` op (the host dropped the page). */
export function clearPage(page: number): void {
  const current = pageRoots.get(page)
  if (!current) return
  pageRoots.delete(page)
  dropRoot(current)
}

function dropRoot(node: ElementNode): void {
  node.page = 0
  if (node.live) {
    touched.add(node)
    schedule()
  }
}
