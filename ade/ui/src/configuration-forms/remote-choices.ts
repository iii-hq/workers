import { useContext, useEffect, useState } from 'react'
import { ConfigurationHostContext } from './model-catalog'
import type { Choice, RemoteChoices } from './types'

const RPC_TIMEOUT_MS = 10_000

/**
 * The choices `source` lists, read once per form mount through the form's
 * `host`. Empty while loading, without a host, or when the worker behind the
 * function is not running: the field then shows its static options and the
 * stored value.
 */
export function useRemoteChoices(source: RemoteChoices | undefined): readonly Choice[] {
  const host = useContext(ConfigurationHostContext)
  const [choices, setChoices] = useState<readonly Choice[]>([])
  useEffect(() => {
    if (!host || !source) {
      setChoices([])
      return
    }
    let live = true
    // `resolve().then` turns a synchronous throw (no connection yet) into the same empty list.
    Promise.resolve()
      .then(() => host.iii.trigger(source.functionId, {}, { timeoutMs: RPC_TIMEOUT_MS }))
      .then((reply) => source.choices(reply))
      .catch(() => [])
      .then((list) => {
        if (live) setChoices(list)
      })
    return () => {
      live = false
    }
  }, [host, source])
  return choices
}

/**
 * Static choices, then remote ones not already listed, then the stored value
 * when neither lists it (a catalog that is down or no longer has it), so a
 * configured value is never shown as invalid just because its source is away.
 */
export function mergeChoices(
  fixed: readonly Choice[],
  remote: readonly Choice[],
  stored: string | undefined,
): readonly Choice[] {
  const seen = new Set(fixed.map((choice) => choice.value))
  const merged = [...fixed]
  for (const choice of remote) {
    if (seen.has(choice.value)) continue
    seen.add(choice.value)
    merged.push(choice)
  }
  if (stored !== undefined && stored !== '' && !seen.has(stored)) {
    merged.push({ value: stored, label: stored, description: 'Not listed by the running worker' })
  }
  return merged
}
