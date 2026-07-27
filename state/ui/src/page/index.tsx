/**
 * The state-manager page: scopes → keys → JSON value editor, all live.
 * One `state` trigger binding for the whole page (src/lib/events.ts),
 * fanned out to whichever view is mounted.
 */

import { useState } from 'react'
import type { Host } from '@iii-dev/console-ui'
import { useStateEventHub } from '../lib/events'
import { ItemsView } from './ItemsView'
import { ItemView } from './ItemView'
import { ScopesView } from './ScopesView'

type Route =
  | { view: 'scopes' }
  | { view: 'items'; scope: string }
  | { view: 'item'; scope: string; key: string }

export function StateManagerPage({ host }: { host: Host }) {
  const [route, setRoute] = useState<Route>({ view: 'scopes' })
  const subscribe = useStateEventHub(host)

  return (
    <div className="state-ui">
      {route.view === 'scopes' ? (
        <ScopesView
          host={host}
          subscribe={subscribe}
          onOpen={(scope) => setRoute({ view: 'items', scope })}
        />
      ) : route.view === 'items' ? (
        <ItemsView
          host={host}
          scope={route.scope}
          subscribe={subscribe}
          onBack={() => setRoute({ view: 'scopes' })}
          onOpen={(key) => setRoute({ view: 'item', scope: route.scope, key })}
        />
      ) : (
        <ItemView
          host={host}
          scope={route.scope}
          itemKey={route.key}
          subscribe={subscribe}
          onBack={() => setRoute({ view: 'items', scope: route.scope })}
        />
      )}
    </div>
  )
}
