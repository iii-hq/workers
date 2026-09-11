import { createContext, type ReactNode, useContext } from 'react'

const ExtensionScopeContext = createContext<string | undefined>(undefined)

export function ExtensionScopeProvider({
  scope,
  children,
}: {
  scope: string
  children: ReactNode
}) {
  return (
    <ExtensionScopeContext.Provider value={scope}>
      {children}
    </ExtensionScopeContext.Provider>
  )
}

export function useExtensionScope(): string | undefined {
  return useContext(ExtensionScopeContext)
}

/**
 * Portalled shared UI remains under the worker's CSS scope. `display:
 * contents` keeps the wrapper out of positioning and layout while preserving
 * descendant selectors generated for `[data-iii-ui="worker"]`.
 */
export function PortalScope({ children }: { children: ReactNode }) {
  const scope = useExtensionScope()
  if (!scope) return children
  return (
    <div data-iii-ui={scope} style={{ display: 'contents' }}>
      {children}
    </div>
  )
}
