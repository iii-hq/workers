/**
 * Create and delete for the state page, kept free of React so the rules are
 * testable on their own (tests/entries.test.ts).
 *
 * The worker has no scope verbs: a scope exists while it holds a key. So
 * "new scope" is a new key in a scope nobody has written yet, and "delete
 * scope" deletes every key in it (the scope leaves `state::list_groups`
 * once the last one goes).
 */

/** The slice of `host.iii` these calls need. */
export interface StateCaller {
  call<T = unknown>(functionId: string, data?: unknown): Promise<T>
}

export interface NewEntryDraft {
  scope: string
  key: string
  /** The value as typed: JSON text. */
  value: string
}

export interface NewEntryErrors {
  scope?: string
  key?: string
  value?: string
}

/** What is wrong with the draft, per field; an empty object means it can be created. */
export function validateNewEntry(draft: NewEntryDraft): NewEntryErrors {
  const errors: NewEntryErrors = {}
  if (draft.scope.trim() === '') errors.scope = 'Name the scope.'
  if (draft.key.trim() === '') errors.key = 'Name the key.'
  try {
    JSON.parse(draft.value)
  } catch (err) {
    errors.value = `Not valid JSON: ${err instanceof Error ? err.message : String(err)}`
  }
  return errors
}

export const KEY_EXISTS = 'A key with this name already exists in this scope.'

/**
 * Write a new key, only if it is absent: `state::compare-and-set` without
 * `expected` swaps only when nothing is stored, so an existing value is never
 * overwritten by "New key". Rejects with {@link KEY_EXISTS} when it is taken.
 */
export async function createEntry(
  iii: StateCaller,
  draft: NewEntryDraft,
): Promise<{ scope: string; key: string }> {
  const errors = validateNewEntry(draft)
  const first = errors.scope ?? errors.key ?? errors.value
  if (first) throw new Error(first)
  const scope = draft.scope.trim()
  const key = draft.key.trim()
  const result = await iii.call<{ swapped: boolean }>('state::compare-and-set', {
    scope,
    key,
    value: JSON.parse(draft.value),
  })
  if (!result?.swapped) throw new Error(KEY_EXISTS)
  return { scope, key }
}

export async function deleteEntry(
  iii: StateCaller,
  scope: string,
  key: string,
): Promise<void> {
  await iii.call('state::delete', { scope, key })
}

/**
 * Delete every key in a scope, one `state::delete` each (each emits its own
 * deleted event, so open views prune live). Lists the keys first rather than
 * trusting the page's copy, so a key written since the list loaded goes too.
 * Returns how many keys were deleted.
 */
export async function deleteScope(iii: StateCaller, scope: string): Promise<number> {
  const { keys } = await iii.call<{ keys: string[] }>('state::list_keys', { scope })
  for (const key of keys) {
    await iii.call('state::delete', { scope, key })
  }
  return keys.length
}
