/**
 * The catalog as lookups, and the json-v0 entries in named form (for test
 * assertions, snapshots and the conformance files).
 */

import { readFileSync } from 'node:fs'

export const catalog = JSON.parse(readFileSync(new URL('../../catalog.json', import.meta.url), 'utf8'))

const byKey = (list, key) => new Map(list.map((x) => [x[key], x]))

export const typesByTag = byKey(catalog.types, 'tag')
export const typesByName = byKey(catalog.types, 'name')
export const eventsByTag = byKey(catalog.events, 'tag')
export const eventsByName = byKey(catalog.events, 'name')
export const vmToHostByCode = byKey(catalog.protocol.vmToHost, 'code')
export const hostToVmByCode = byKey(catalog.protocol.hostToVm, 'code')
export const requestsByMethod = byKey(catalog.protocol.requests, 'method')
export const requestsByName = byKey(catalog.protocol.requests, 'name')
const commonByTag = byKey(catalog.commonProps, 'tag')
const commonByName = byKey(catalog.commonProps, 'name')

/** The prop spec of `tag` on type `typeTag` (its own props, then the common ones). */
export function propOf(typeTag, tag) {
  const type = typesByTag.get(typeTag)
  return type?.props.find((p) => p.tag === tag) ?? commonByTag.get(tag) ?? null
}

export function propByName(typeTag, name) {
  const type = typesByTag.get(typeTag)
  return type?.props.find((p) => p.name === name) ?? commonByName.get(name) ?? null
}

export function slotName(typeTag, slot) {
  return typesByTag.get(typeTag)?.slots.find((s) => s.slot === slot)?.name ?? `slot${slot}`
}

export function slotSpec(typeTag, slot) {
  return typesByTag.get(typeTag)?.slots.find((s) => s.slot === slot) ?? null
}

export function eventNames(mask) {
  const names = []
  for (const e of catalog.events) if (mask & (1 << e.tag)) names.push(e.name)
  return names
}

/** A prop value in readable form: enum indexes as names, the events mask as event names. */
export function readableValue(prop, value) {
  if (!prop || value === null || value === undefined) return value
  if (prop.type === 'enum' && Number.isInteger(value)) return catalog.enums[prop.enum][value] ?? value
  if (prop.name === 'events' && Number.isInteger(value)) return eventNames(value)
  return value
}

/**
 * A stateful decoder of json-v0 messages into named entries; it remembers
 * the type of each created node to name the props and slots of later ops.
 */
export function createDecoder() {
  const types = new Map()
  const named = (id) => types.get(id)
  const fromVm = (e) => {
    switch (e[0]) {
      case 1:
        types.set(e[1], e[2])
        return { op: 'create', id: e[1], type: typesByTag.get(e[2])?.name ?? e[2] }
      case 2: {
        const prop = propOf(named(e[1]), e[2])
        const out = { op: 'set', id: e[1], prop: prop?.name ?? e[2], value: e[3] }
        const readable = readableValue(prop, e[3])
        if (readable !== e[3]) out.readable = readable
        return out
      }
      case 3:
        return { op: 'insert', parent: e[1], slot: slotName(named(e[1]), e[2]), index: e[3], child: e[4] }
      case 4:
        return { op: 'move', parent: e[1], slot: slotName(named(e[1]), e[2]), from: e[3], to: e[4] }
      case 5:
        return { op: 'remove', parent: e[1], slot: slotName(named(e[1]), e[2]), index: e[3] }
      case 6:
        return { op: 'destroy', id: e[1] }
      case 7:
        return { op: 'root', page: e[1], id: e[2] }
      case 8:
        return { op: 'listSplice', list: e[1], start: e[2], deleteCount: e[3], items: e[4] }
      case 20:
        return { op: 'request', reqId: e[1], method: requestsByMethod.get(e[2])?.name ?? e[2], args: e[3] }
      case 23:
        return { op: 'ready', info: e[1] }
      default:
        return { op: 'unknown', entry: e }
    }
  }
  const toVm = (e) => {
    switch (e[0]) {
      case 1:
        return { op: 'init', info: e[1] }
      case 2:
        return { op: 'event', node: e[1], event: eventsByTag.get(e[2])?.name ?? e[2], args: e[3] }
      case 3:
        return {
          op: 'itemEvent',
          list: e[1],
          key: e[2],
          node: e[3],
          event: eventsByTag.get(e[4])?.name ?? e[4],
          args: e[5],
        }
      case 4:
        return { op: 'text', node: e[1], count: e[2], text: e[3] }
      case 5:
        return { op: 'env', changes: e[1] }
      case 6:
        return { op: 'mount', page: e[1], name: e[2], params: e[3] }
      case 7:
        return { op: 'unmount', page: e[1] }
      case 8:
        return { op: 'response', reqId: e[1], ok: e[2] === 1, value: e[3] }
      case 9:
        return { op: 'trigger', sub: e[1], payload: e[2] }
      default:
        return { op: 'unknown', entry: e }
    }
  }
  return {
    /**
     * `direction` is 'fromVm' (VM → host) or 'toVm' (host → VM); `context`
     * names the types of nodes created elsewhere (`{"20": "TextField"}`).
     */
    decode(message, direction, context) {
      for (const [id, type] of Object.entries(context ?? {})) types.set(Number(id), typesByName.get(type)?.tag)
      const entries = typeof message === 'string' ? JSON.parse(message) : message
      return entries.map(direction === 'toVm' ? toVm : fromVm)
    },
  }
}

/** Decodes one message without context (set/insert of nodes it does not create keep numeric tags). */
export function decodeMessage(message, direction) {
  return createDecoder().decode(message, direction)
}

/** Checks a wire value against a prop's catalog type; returns a problem or null. */
export function checkValue(prop, value) {
  if (value === null) return null
  if (typeof value === 'object' && !Array.isArray(value) && typeof value[catalog.binding.key] === 'string') return null
  const isInt = (v) => Number.isInteger(v) && v >= -2147483648 && v <= 2147483647
  switch (prop.type) {
    case 'bool':
      return typeof value === 'boolean' ? null : 'not a bool'
    case 'int':
      return isInt(value) ? null : 'not an int'
    case 'float':
    case 'dp':
      return typeof value === 'number' && Number.isFinite(value) ? null : 'not a number'
    case 'string':
    case 'icon':
      return typeof value === 'string' ? null : 'not a string'
    case 'strings':
      return Array.isArray(value) && value.every((v) => typeof v === 'string') ? null : 'not an array of strings'
    case 'padding':
      return typeof value === 'number' ||
        (Array.isArray(value) &&
          (value.length === 2 || value.length === 4) &&
          value.every((v) => typeof v === 'number'))
        ? null
        : 'not a padding'
    case 'enum':
      return isInt(value) && value >= 0 && value < catalog.enums[prop.enum].length
        ? null
        : `not an index of ${prop.enum}`
    case 'textWrite':
      return Array.isArray(value) && value.length === 2 && typeof value[0] === 'string' && isInt(value[1])
        ? null
        : 'not [text, count]'
    default:
      return null
  }
}
