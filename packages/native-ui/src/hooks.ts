/** Solid hooks over the capability object (design §10.4). */

import { createEffect, createResource, onCleanup, type Resource, useContext } from 'solid-js'
import { PageContext, type PageRef } from './app'
import { type CallOptions, type Env, iii, type Nav, type ObserveSource } from './iii'
import type { JsonObject, JsonValue } from './types'

/**
 * Calls `fn` (of the app's namespace) as a Solid resource. It calls again
 * whenever `payload` changes; `payload` returning undefined (or `enabled`
 * false) means "do not call now". Without `payload`, it calls once with no
 * payload.
 */
export function useCall<T>(
  fn: string,
  payload?: () => JsonValue | undefined,
  opts?: CallOptions & { enabled?: () => boolean },
): [Resource<T | undefined>, { refetch(): void; mutate(value: T): void }] {
  const source = () => {
    if (opts?.enabled && !opts.enabled()) return undefined
    if (!payload) return { payload: undefined }
    const p = payload()
    return p === undefined ? undefined : { payload: p }
  }
  const [resource, { refetch, mutate }] = createResource(source, (s) =>
    iii.call<T>(fn, s.payload, opts?.timeoutMs === undefined ? undefined : { timeoutMs: opts.timeoutMs }),
  )
  return [
    resource as Resource<T | undefined>,
    {
      refetch: () => {
        refetch()
      },
      mutate: (value: T) => {
        mutate(() => value)
      },
    },
  ]
}

/**
 * Observes a function of the app's namespace while the calling scope lives;
 * `source` returning null (or `enabled` false) disposes the observer, and a
 * new source observes again.
 */
export function useObserve<P = JsonValue>(
  functionId: string,
  source: () => ObserveSource | null,
  handler: (payload: P) => void,
  opts?: { enabled?: () => boolean },
): void {
  createEffect(() => {
    if (opts?.enabled && !opts.enabled()) return
    const s = source()
    if (s === null) return
    onCleanup(iii.on<P>(functionId, s, (payload) => handler(payload)))
  })
}

/** The host's environment as a reactive store. */
export function useEnv(): Readonly<Env> {
  return iii.env
}

export function useNav(): Nav {
  return iii.nav
}

/** The page instance being rendered, with `pop` and `replace` bound to it. */
export function usePage(): PageRef & {
  pop(result?: JsonValue): void
  replace(name: string, params?: JsonObject): void
} {
  const page = useContext(PageContext)
  if (!page) throw new Error('usePage is only available inside a page component')
  return {
    inst: page.inst,
    name: page.name,
    params: page.params,
    pop: (result) => iii.nav.pop(page, result),
    replace: (name, params) => iii.nav.replace(page, name, params),
  }
}
