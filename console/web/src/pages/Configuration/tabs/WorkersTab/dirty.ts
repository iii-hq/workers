/**
 * Deep structural comparison for `JsonValue` trees plus a small helper
 * that the editor uses to decide whether to show the save bar.
 *
 * Why a custom comparator instead of `JSON.stringify`-then-compare:
 *
 * - Object key order shouldn't affect dirtiness. Forms reorder keys all
 *   the time (e.g. spreading `{ ...obj, k: v }`), and we don't want
 *   that noise to flip the dirty bit.
 * - `null` and `undefined` and missing keys should compare equal, because
 *   the schema form coerces unset → `null` while the loaded value may
 *   omit the key entirely. A naive comparison would mark every loaded
 *   value as dirty the first time the operator touched any field.
 *
 * The function never throws and always terminates: it never recurses on
 * the same identical pair twice (short-circuit on `Object.is`), and the
 * structural recursion is bounded by tree depth.
 */

import type { JsonValue } from './api'

/**
 * Treat `null`, `undefined`, and missing-key as the same. Returns true
 * if neither value carries meaningful content for the configuration.
 */
function isAbsent(value: JsonValue | undefined): boolean {
  return value === null || value === undefined
}

/**
 * Structural deep-equal for JSON-shaped values. Key order is ignored
 * for objects; null / undefined / missing keys collapse together.
 */
export function jsonEqual(
  a: JsonValue | undefined,
  b: JsonValue | undefined,
): boolean {
  if (Object.is(a, b)) return true
  if (isAbsent(a) && isAbsent(b)) return true
  if (isAbsent(a) || isAbsent(b)) return false

  const aIsArray = Array.isArray(a)
  const bIsArray = Array.isArray(b)
  if (aIsArray !== bIsArray) return false

  if (aIsArray && bIsArray) {
    if (a.length !== b.length) return false
    for (let i = 0; i < a.length; i++) {
      if (!jsonEqual(a[i], b[i])) return false
    }
    return true
  }

  if (typeof a === 'object' && typeof b === 'object') {
    const aObj = a as { [key: string]: JsonValue }
    const bObj = b as { [key: string]: JsonValue }
    // Union of keys so we catch "key present in one, missing in the
    // other" — which still equals when both sides are absent-ish.
    const seen = new Set<string>()
    for (const key of Object.keys(aObj)) {
      seen.add(key)
      if (!jsonEqual(aObj[key], bObj[key])) return false
    }
    for (const key of Object.keys(bObj)) {
      if (seen.has(key)) continue
      if (!jsonEqual(aObj[key], bObj[key])) return false
    }
    return true
  }

  return a === b
}

/**
 * Returns true when the working draft differs structurally from the
 * loaded baseline. Thin wrapper for naming — `isDirty(loaded, draft)`
 * reads more clearly at call sites than `!jsonEqual(loaded, draft)`.
 */
export function isDirty(
  loaded: JsonValue | undefined,
  draft: JsonValue | undefined,
): boolean {
  return !jsonEqual(loaded, draft)
}
