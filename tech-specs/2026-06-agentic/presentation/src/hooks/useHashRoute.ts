import { useEffect, useState } from 'react'

export type Route =
  | { kind: 'home' }
  | { kind: 'use-case'; id: 'telegram' | 'console' | 'loops' }

function parse(hash: string): Route {
  const m = hash.match(/^#\/use-cases\/(telegram|console|loops)$/)
  if (m) return { kind: 'use-case', id: m[1] as 'telegram' | 'console' | 'loops' }
  return { kind: 'home' }
}

/**
 * hash routing with two namespaces: `#/...` paths are routes (deep-dive
 * pages); bare `#section-id` hashes stay native anchor scrolls on the home
 * page.
 */
export function useHashRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parse(window.location.hash))

  useEffect(() => {
    const onChange = () => setRoute(parse(window.location.hash))
    window.addEventListener('hashchange', onChange)
    return () => window.removeEventListener('hashchange', onChange)
  }, [])

  useEffect(() => {
    // deep-dive pages and the explicit "#/" home link start at the top;
    // bare "#section" hashes keep native anchor behaviour.
    if (route.kind === 'use-case' || window.location.hash === '#/') {
      window.scrollTo({ top: 0, behavior: 'instant' })
    }
  }, [route])

  return route
}
