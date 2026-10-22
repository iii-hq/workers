/**
 * The host side of the tree in the test harness: applies VM → host ops the
 * way the app's UiTree does (design §5.3, §5.5, §5.6), checks the
 * guarantees the SDK gives (create before use, inserts of detached nodes,
 * indexes in range, no node left detached at the end of a batch, bindings
 * only in templates), and gives the tree back in named form.
 */

import { catalog, checkValue, propOf, readableValue, slotName, slotSpec, typesByName, typesByTag } from './protocol.mjs'

const TEXT_FIELD = typesByName.get('TextField').tag
const LIST = typesByName.get('List').tag
const BINDING = catalog.binding.key

export class HostTree {
  /** @param {(problem: string) => void} violation */
  constructor(violation) {
    this.violation = violation
    /** id → { id, type, props: Map, slots: Map<slot, id[]>, parent: {id, slot} | null, items?, text?, count? } */
    this.nodes = new Map()
    /** page instance → root id */
    this.roots = new Map()
    /** Writes into a TextField the host dropped (the person typed after the script's count). */
    this.droppedWrites = []
    this.touched = new Set()
  }

  node(id, op) {
    const n = this.nodes.get(id)
    if (!n) this.violation(`${op}: node ${id} does not exist`)
    return n
  }

  apply(e) {
    switch (e[0]) {
      case 1: {
        const [, id, type] = e
        if (!Number.isInteger(id) || id <= 0) return this.violation(`create: bad id ${id}`)
        if (this.nodes.has(id)) return this.violation(`create: node ${id} exists`)
        if (!typesByTag.has(type)) this.violation(`create: unknown type ${type} (the host draws Unknown)`)
        const n = { id, type, props: new Map(), slots: new Map(), parent: null }
        if (type === TEXT_FIELD) {
          n.text = ''
          n.count = 0
        }
        if (type === LIST) n.items = []
        this.nodes.set(id, n)
        this.touched.add(id)
        return
      }
      case 2: {
        const [, id, tag, value] = e
        const n = this.node(id, 'set')
        if (!n) return
        const prop = propOf(n.type, tag)
        if (!prop) return this.violation(`set: ${typesByTag.get(n.type)?.name} has no prop ${tag}`)
        if (value === undefined) return this.violation(`set: no value for ${prop.name} of node ${id}`)
        const problem = checkValue(prop, value)
        if (problem) return this.violation(`set: ${prop.name} of node ${id} is ${problem}: ${JSON.stringify(value)}`)
        if (n.type === TEXT_FIELD && tag === 16) return this.writeText(n, value)
        if (value === null) n.props.delete(tag)
        else n.props.set(tag, value)
        if (value && typeof value === 'object' && !Array.isArray(value) && BINDING in value) this.touched.add(id)
        return
      }
      case 3: {
        const [, parentId, slot, index, childId] = e
        const parent = this.node(parentId, 'insert')
        const child = this.node(childId, 'insert')
        if (!parent || !child) return
        if (child.parent) return this.violation(`insert: node ${childId} is attached to ${child.parent.id}`)
        if ([...this.roots.values()].includes(childId)) return this.violation(`insert: node ${childId} is a page root`)
        if (!slotSpec(parent.type, slot)) {
          return this.violation(`insert: ${typesByTag.get(parent.type)?.name} has no slot ${slot}`)
        }
        const list = this.slot(parent, slot)
        if (!Number.isInteger(index) || index < 0 || index > list.length) {
          return this.violation(`insert: index ${index} out of 0..${list.length} in ${parentId}/${slot}`)
        }
        if (this.isAncestor(childId, parent)) return this.violation(`insert: node ${childId} would contain itself`)
        list.splice(index, 0, childId)
        child.parent = { id: parentId, slot }
        return
      }
      case 4: {
        const [, parentId, slot, from, to] = e
        const parent = this.node(parentId, 'move')
        if (!parent) return
        const list = this.slot(parent, slot)
        if (!(from >= 0 && from < list.length && to >= 0 && to < list.length)) {
          return this.violation(`move: ${from} → ${to} out of range in ${parentId}/${slot} (${list.length})`)
        }
        const [id] = list.splice(from, 1)
        list.splice(to, 0, id)
        return
      }
      case 5: {
        const [, parentId, slot, index] = e
        const parent = this.node(parentId, 'remove')
        if (!parent) return
        const list = this.slot(parent, slot)
        if (!(index >= 0 && index < list.length)) {
          return this.violation(`remove: index ${index} out of range in ${parentId}/${slot} (${list.length})`)
        }
        const [id] = list.splice(index, 1)
        const child = this.nodes.get(id)
        if (child) child.parent = null
        this.touched.add(id)
        return
      }
      case 6: {
        const [, id] = e
        const n = this.node(id, 'destroy')
        if (!n) return
        if (n.parent) this.violation(`destroy: node ${id} is still attached to ${n.parent.id}`)
        if ([...this.roots.values()].includes(id)) this.violation(`destroy: node ${id} is a page root`)
        this.free(n)
        return
      }
      case 7: {
        const [, page, id] = e
        if (!Number.isInteger(page) || page <= 0) return this.violation(`root: bad page ${page}`)
        const old = this.roots.get(page)
        if (id === 0) {
          this.roots.delete(page)
        } else {
          const n = this.node(id, 'root')
          if (!n) return
          if (n.parent) return this.violation(`root: node ${id} is attached to ${n.parent.id}`)
          this.roots.set(page, id)
        }
        if (old !== undefined && old !== id) this.touched.add(old)
        return
      }
      case 8: {
        const [, id, start, deleteCount, items] = e
        const n = this.node(id, 'listSplice')
        if (!n) return
        if (n.type !== LIST) return this.violation(`listSplice: node ${id} is not a List`)
        if (!Array.isArray(items)) return this.violation('listSplice: items is not an array')
        if (!(start >= 0 && start <= n.items.length && deleteCount >= 0 && start + deleteCount <= n.items.length)) {
          return this.violation(`listSplice: ${start}+${deleteCount} out of range (${n.items.length})`)
        }
        if (items.some((it) => !it || typeof it !== 'object' || Array.isArray(it))) {
          return this.violation('listSplice: every item is a JSON object')
        }
        n.items.splice(start, deleteCount, ...items)
        const keyField = n.props.get(16) ?? 'key'
        const keys = n.items.map((it) => it[keyField])
        if (new Set(keys).size !== keys.length) this.violation(`listSplice: duplicate keys in List ${id}`)
        return
      }
      default:
        this.violation(`unknown VM → host code ${e[0]}`)
    }
  }

  /** The guarantees at the end of a batch: no node left detached, bindings only inside templates. */
  endOfMessage() {
    const roots = new Set(this.roots.values())
    for (const id of this.touched) {
      const n = this.nodes.get(id)
      if (!n) continue
      if (!n.parent && !roots.has(id)) this.violation(`node ${id} ended the batch detached without a destroy`)
      for (const [tag, value] of n.props) {
        if (value && typeof value === 'object' && !Array.isArray(value) && BINDING in value && !this.inTemplate(n)) {
          this.violation(`node ${id}: ${propOf(n.type, tag)?.name} is bound outside a List template`)
        }
      }
    }
    this.touched.clear()
  }

  writeText(n, value) {
    const [text, count] = value
    if (count === n.count) n.text = text
    else this.droppedWrites.push({ node: n.id, text, count, hostCount: n.count, hostText: n.text })
  }

  /** The person typed `text` into a TextField: the host's count goes up. */
  edit(id, text) {
    const n = this.nodes.get(id)
    n.count += 1
    n.text = text
    return n.count
  }

  slot(n, slot) {
    let list = n.slots.get(slot)
    if (!list) {
      list = []
      n.slots.set(slot, list)
    }
    return list
  }

  isAncestor(id, of) {
    for (let cur = of; cur; cur = cur.parent ? this.nodes.get(cur.parent.id) : null) if (cur.id === id) return true
    return false
  }

  free(n) {
    this.nodes.delete(n.id)
    if (n.parent) {
      const list = this.nodes.get(n.parent.id)?.slots.get(n.parent.slot)
      if (list) list.splice(list.indexOf(n.id), 1)
    }
    for (const list of n.slots.values())
      for (const id of list) {
        const c = this.nodes.get(id)
        if (c) {
          c.parent = null
          this.free(c)
        }
      }
  }

  /** Whether the node sits inside a List template (slots 1–4 of a List). */
  inTemplate(n) {
    for (let cur = n; cur?.parent; cur = this.nodes.get(cur.parent.id)) {
      const parent = this.nodes.get(cur.parent.id)
      if (parent && slotSpec(parent.type, cur.parent.slot)?.template) return true
    }
    return false
  }

  /** The nodes attached under page roots, in tree order; templates only with `templates: true`. */
  *walk({ templates = false } = {}) {
    const visit = function* (tree, id, inTemplate) {
      const n = tree.nodes.get(id)
      if (!n) return
      if (!inTemplate || templates) yield n
      for (const [slot, list] of [...n.slots].sort((a, b) => a[0] - b[0])) {
        const t = inTemplate || Boolean(slotSpec(n.type, slot)?.template)
        for (const c of list) yield* visit(tree, c, t)
      }
    }
    for (const id of this.roots.values()) yield* visit(this, id, false)
  }

  /** One node and its subtree in named form. */
  view(id) {
    const n = this.nodes.get(id)
    if (!n) return null
    const out = { type: typesByTag.get(n.type)?.name ?? `Unknown(${n.type})`, id }
    const props = {}
    for (const [tag, value] of [...n.props].sort((a, b) => a[0] - b[0])) {
      const prop = propOf(n.type, tag)
      props[prop?.name ?? tag] = readableValue(prop, value)
    }
    if (n.type === TEXT_FIELD) props.value = n.text
    if (Object.keys(props).length) out.props = props
    const slots = {}
    for (const [slot, list] of [...n.slots].sort((a, b) => a[0] - b[0])) {
      if (list.length) slots[slotName(n.type, slot)] = list.map((c) => this.view(c))
    }
    if (Object.keys(slots).length) out.slots = slots
    if (n.items) out.items = n.items
    return out
  }
}
