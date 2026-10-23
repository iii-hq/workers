/**
 * Templated lists (design §5.5, §10.3). A `List` has no node per item: it
 * has up to four template subtrees whose props may be bound to item fields
 * (`{"$item": "path"}`), and its items live on the host, kept in sync with
 * `listSplice`. `createList` is the data source; one source can feed the
 * List of any page.
 */

import { createRenderEffect, createSignal, type JSX, onCleanup, untrack } from 'solid-js'
import { applyProps, applySlots } from './components'
import { ENUMS, type EnumName, LIMITS, OP } from './generated/catalog'
import type { ListBaseProps } from './generated/props'
import { insert } from './renderer'
import { diffItems } from './runtime/diff'
import {
  createNode,
  ELEMENT,
  type ElementNode,
  isBinding,
  nodeById,
  propSpecOf,
  setNamedProp,
  slotContainer,
  warnOnce,
} from './runtime/tree'
import { push } from './runtime/wire'
import type { Bound } from './types'

export interface ListSource<T extends object> {
  /** The item field holding its key (a string or a number, unique). */
  readonly keyField: string
  /** The item field holding 0..3: which template draws it; null = always the first. */
  readonly kindField: string | null
  /** The items now (reactive). Treat them as immutable: change them through this source. */
  items(): readonly T[]
  /** Replaces every item; only the positions that changed go to the host. */
  set(items: readonly T[]): void
  /** Array.prototype.splice over the items. */
  splice(start: number, deleteCount: number, ...items: T[]): T[]
  /** Merges `patch` into the item with `key`; false when there is none. */
  update(key: unknown, patch: Partial<T>): boolean
  /** Removes the item with `key`; false when there is none. */
  remove(key: unknown): boolean
  get(key: unknown): T | undefined
}

const VIEWS = Symbol('views')
const RAW = Symbol('raw')

export function createList<T extends object>(options: {
  key: keyof T & string
  kind?: keyof T & string
  items?: readonly T[]
}): ListSource<T> {
  const keyField = options.key
  const kindField = options.kind ?? null
  let items: readonly T[] = [...(options.items ?? [])]
  let index: Map<unknown, number> | null = null
  const [version, bump] = createSignal(0)
  const views = new Set<ElementNode>()
  const keyOf = (item: T) => (item as Record<string, unknown>)[keyField]

  const apply = (start: number, deleteCount: number, added: T[]): T[] => {
    const next = items.slice()
    const removed = next.splice(start, deleteCount, ...added)
    if (next.length > LIMITS.maxListItems) warnOnce(`a List has more than ${LIMITS.maxListItems} items`)
    items = next
    index = null
    for (const node of views) if (node.live) push([OP.listSplice, node.id, start, deleteCount, wireItems(node, added)])
    return removed
  }
  const changed = () => bump((v) => v + 1)
  const find = (key: unknown): number => {
    if (!index) {
      const map = new Map<unknown, number>()
      for (let i = 0; i < items.length; i++) map.set(keyOf(items[i]), i)
      index = map
    }
    return index.get(key) ?? -1
  }

  const source: ListSource<T> & { [VIEWS]: Set<ElementNode>; [RAW]: () => readonly T[] } = {
    keyField,
    kindField,
    items() {
      version()
      return items
    },
    set(all) {
      if (new Set(all.map(keyOf)).size !== all.length) {
        warnOnce(`a List got the same ${keyField} twice; pass its items through lastPerKey`)
      }
      const ops = diffItems(items, all, keyOf)
      for (const [start, del, added] of ops) apply(start, del, added)
      if (ops.length) changed()
    },
    splice(start, deleteCount, ...added) {
      const len = items.length
      const s = start < 0 ? Math.max(len + start, 0) : Math.min(start, len)
      const d = Math.min(Math.max(deleteCount, 0), len - s)
      if (!d && !added.length) return []
      const removed = apply(s, d, added)
      changed()
      return removed
    },
    update(key, patch) {
      const i = find(key)
      if (i < 0) return false
      const next = { ...items[i], ...patch }
      if (JSON.stringify(next) !== JSON.stringify(items[i])) {
        apply(i, 1, [next])
        changed()
      }
      return true
    },
    remove(key) {
      const i = find(key)
      if (i < 0) return false
      apply(i, 1, [])
      changed()
      return true
    },
    get(key) {
      const i = find(key)
      return i < 0 ? undefined : items[i]
    },
    [VIEWS]: views,
    [RAW]: () => items,
  }
  return source
}

type Internal<T extends object> = ListSource<T> & { [VIEWS]: Set<ElementNode>; [RAW]: () => readonly T[] }

export interface ListProps<T extends object> extends Omit<ListBaseProps, 'children'> {
  source: ListSource<T>
  /**
   * Up to four row templates, each called once with `row`, whose fields are
   * bindings to the item's fields. Handlers inside a template get the item
   * as their first argument.
   */
  templates: ReadonlyArray<(row: Bound<T>) => JSX.Element>
}

/** A binding to `path` that also binds deeper paths (`row.a.b`). */
function binding(path: string): unknown {
  return new Proxy(
    {},
    {
      get(_, key) {
        if (key === '$item') return path
        if (key === 'toJSON') return () => ({ $item: path })
        if (typeof key === 'symbol') return undefined
        return binding(`${path}.${key}`)
      },
    },
  )
}

function rowProxy<T>(): Bound<T> {
  return new Proxy(
    {},
    {
      get(_, key) {
        if (typeof key === 'symbol' || key === 'toJSON' || key === 'then') return undefined
        return binding(key)
      },
    },
  ) as Bound<T>
}

/** The item fields the templates of `node` bind to an enum prop: path → the enum. */
function enumFields(node: ElementNode): Map<string, EnumName> {
  const fields = new Map<string, EnumName>()
  const visit = (n: ElementNode) => {
    for (const [tag, value] of n.props) {
      if (!isBinding(value)) continue
      const spec = propSpecOf(n, tag)
      if (spec?.[1] !== 'enum') continue
      const enumName = spec[2] as EnumName
      const had = fields.get(value.$item)
      if (had && had !== enumName) {
        warnOnce(`List: the item field ${value.$item} is bound to a ${had} and a ${enumName} prop; it goes as a ${had}`)
      } else fields.set(value.$item, enumName)
    }
    for (const c of n.children) if (c.kind === ELEMENT) visit(c)
    for (const slot of n.slots) if (slot) for (const c of slot.children) if (c.kind === ELEMENT) visit(c)
  }
  for (const [, [slot, , template]] of Object.entries(node.spec.slots)) {
    if (!template) continue
    for (const c of node.slots[slot]?.children ?? []) if (c.kind === ELEMENT) visit(c)
  }
  return fields
}

/**
 * The items as the host reads them. A field bound to an enum prop
 * (`hue={row.hue}`) may hold the enum's name, like a plain prop: the host
 * reads an index, so the name goes as its index (an unknown name as null,
 * the prop's default). The script's items keep the names.
 */
function wireItems(node: ElementNode, items: readonly unknown[]): unknown[] {
  const fields = enumFields(node)
  if (!fields.size) return items.slice()
  return items.map((item) => {
    let out = item
    for (const [path, enumName] of fields) out = withEnumIndex(out, path.split('.'), enumName, path)
    return out
  })
}

function withEnumIndex(value: unknown, keys: string[], enumName: EnumName, path: string): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const [key, ...rest] = keys
  const current = (value as Record<string, unknown>)[key]
  let next: unknown
  if (rest.length) next = withEnumIndex(current, rest, enumName, path)
  else if (typeof current === 'string') {
    const values = ENUMS[enumName] as readonly string[]
    const index = values.indexOf(current)
    if (index < 0)
      warnOnce(`List: ${JSON.stringify(current)} in the item field ${path} is not one of ${values.join(', ')}`)
    next = index < 0 ? null : index
  } else return value
  return next === current ? value : { ...(value as object), [key]: next }
}

const LIST_OWN = new Set(['source', 'templates', 'onDrop'])

export function List<T extends object>(props: ListProps<T>): JSX.Element {
  const node = createNode('List')
  const source = untrack(() => props.source) as Internal<T>
  if (!source?.[VIEWS]) throw new Error('List needs a source made by createList')
  setNamedProp(node, 'keyField', source.keyField)
  if (source.kindField) setNamedProp(node, 'kindField', source.kindField)
  applyProps(node, props as unknown as Record<string, unknown>, LIST_OWN)
  node.list = { snapshot: source[RAW], get: (key) => source.get(key), detach: () => source[VIEWS].delete(node) }
  node.onLive = () => {
    const items = source[RAW]()
    if (items.length) push([OP.listSplice, node.id, 0, 0, wireItems(node, items)])
  }
  source[VIEWS].add(node)
  onCleanup(() => source[VIEWS].delete(node))
  const p = props as unknown as Record<string, unknown>
  if ('onDrop' in p) {
    // The wire's source is a node id; the handler gets that List's node, the
    // object its `ref` gave (null when it is gone).
    createRenderEffect(() => {
      const onDrop = p.onDrop as ListBaseProps['onDrop']
      setNamedProp(
        node,
        'onDrop',
        typeof onDrop === 'function'
          ? (key: string | number, index: number, from: unknown) => onDrop(key, index, nodeById(from) ?? null)
          : undefined,
      )
    })
  }

  const templates = untrack(() => props.templates) ?? []
  if (templates.length > 4) throw new Error('a List has at most 4 templates')
  templates.forEach((template, i) => {
    insert(
      slotContainer(node, 1 + i),
      untrack(() => template(rowProxy<T>())),
    )
  })
  applySlots(node, props as unknown as Record<string, unknown>)
  return node as unknown as JSX.Element
}
