import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { useWorkerLifecycle } from '@/hooks/use-worker-lifecycle'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import {
  CONSOLE_EXTENSION_API_VERSION,
  CONSOLE_EXTENSION_FUNCTION_SUFFIX,
  type ConsoleExtensionHost,
  type ConsoleExtensionManifest,
  type ConsoleExtensionModule,
  type ConsoleExtensionSlotContribution,
  consoleExtensionAssetSchema,
  consoleExtensionManifestSchema,
  isConsoleExtensionCapability,
  verifyExtensionAsset,
} from './contract'

interface LoadedExtension {
  manifestFunction: string
  manifest: ConsoleExtensionManifest
  dispose(): void
}

interface ConsoleExtensionsContextValue {
  revision: number
  loadedIds: ReadonlySet<string>
  contributions: ReadonlyMap<
    string,
    readonly ConsoleExtensionSlotContribution[]
  >
}

const ConsoleExtensionsContext = createContext<ConsoleExtensionsContextValue>({
  revision: 0,
  loadedIds: new Set(),
  contributions: new Map(),
})

interface RegistryState {
  contributions: Map<string, ConsoleExtensionSlotContribution[]>
  loaded: Map<string, LoadedExtension>
}

export function ConsoleExtensionsProvider({
  children,
}: {
  children: ReactNode
}) {
  const registryRef = useRef<RegistryState>({
    contributions: new Map(),
    loaded: new Map(),
  })
  const [revision, setRevision] = useState(0)
  const generationRef = useRef(0)

  const bump = useCallback(() => setRevision((value) => value + 1), [])

  const registerSlot = useCallback(
    (contribution: ConsoleExtensionSlotContribution): (() => void) => {
      const registry = registryRef.current.contributions
      const rows = registry.get(contribution.slot) ?? []
      if (rows.some((row) => row.id === contribution.id)) {
        throw new Error(
          `duplicate console extension contribution ${contribution.id}`,
        )
      }
      registry.set(
        contribution.slot,
        [...rows, contribution].sort((a, b) => (a.order ?? 0) - (b.order ?? 0)),
      )
      bump()
      let active = true
      return () => {
        if (!active) return
        active = false
        const current = registry.get(contribution.slot) ?? []
        const next = current.filter((row) => row.id !== contribution.id)
        if (next.length > 0) registry.set(contribution.slot, next)
        else registry.delete(contribution.slot)
        bump()
      }
    },
    [bump],
  )

  const refresh = useCallback(async () => {
    const generation = ++generationRef.current
    const client = await getIiiClient()
    const response = await client.trigger<{
      functions?: Array<{ function_id?: unknown }>
    }>('engine::functions::list', { include_internal: true })
    if (generation !== generationRef.current) return

    const candidates = (
      Array.isArray(response?.functions) ? response.functions : []
    )
      .map((row) => row?.function_id)
      .filter(
        (id): id is string =>
          typeof id === 'string' &&
          id.endsWith(CONSOLE_EXTENSION_FUNCTION_SUFFIX),
      )
    const manifestFunctions = new Set(
      (
        await Promise.all(
          candidates.map(async (functionId) => {
            try {
              const info = await client.trigger('engine::functions::info', {
                function_id: functionId,
              })
              return isConsoleExtensionCapability(info, functionId)
                ? functionId
                : null
            } catch (error) {
              console.warn(
                `[console-extension] failed to verify ${functionId}`,
                error,
              )
              return null
            }
          }),
        )
      ).filter((id): id is string => id !== null),
    )
    if (generation !== generationRef.current) return

    for (const [id, loaded] of registryRef.current.loaded) {
      if (manifestFunctions.has(loaded.manifestFunction)) continue
      loaded.dispose()
      registryRef.current.loaded.delete(id)
      bump()
    }

    for (const manifestFunction of manifestFunctions) {
      if (
        [...registryRef.current.loaded.values()].some(
          (loaded) => loaded.manifestFunction === manifestFunction,
        )
      ) {
        continue
      }
      try {
        const loaded = await loadExtension(
          client,
          manifestFunction,
          registerSlot,
        )
        if (generation !== generationRef.current) {
          loaded.dispose()
          return
        }
        const previous = registryRef.current.loaded.get(loaded.manifest.id)
        previous?.dispose()
        registryRef.current.loaded.set(loaded.manifest.id, loaded)
        bump()
      } catch (error) {
        console.error(
          `[console-extension] failed to load ${manifestFunction}`,
          error,
        )
      }
    }
  }, [bump, registerSlot])

  useEffect(() => {
    void refresh().catch((error) => {
      console.warn('[console-extension] initial discovery failed', error)
    })
    return () => {
      generationRef.current++
      for (const loaded of registryRef.current.loaded.values()) {
        loaded.dispose()
      }
      registryRef.current.loaded.clear()
      registryRef.current.contributions.clear()
    }
  }, [refresh])

  useEffect(() => {
    let disposed = false
    let removeConnectionListener: (() => void) | undefined

    void getIiiClient().then((client) => {
      if (disposed) return
      removeConnectionListener = client.addConnectionStateListener((state) => {
        if (state !== 'connected') return
        void refresh().catch((error) => {
          console.warn('[console-extension] reconnect refresh failed', error)
        })
      })
    })

    return () => {
      disposed = true
      removeConnectionListener?.()
    }
  }, [refresh])

  useWorkerLifecycle({
    enabled: true,
    fnId: 'console::extension-worker-watch',
    operations: ['add', 'remove'],
    stages: ['done'],
    onEvent: () => {
      void refresh().catch((error) => {
        console.warn('[console-extension] lifecycle refresh failed', error)
      })
    },
  })

  const value = useMemo<ConsoleExtensionsContextValue>(
    () => ({
      revision,
      loadedIds: new Set(registryRef.current.loaded.keys()),
      contributions: new Map(
        [...registryRef.current.contributions].map(([slot, rows]) => [
          slot,
          [...rows],
        ]),
      ),
    }),
    [revision],
  )

  return (
    <ConsoleExtensionsContext.Provider value={value}>
      {children}
    </ConsoleExtensionsContext.Provider>
  )
}

export function useConsoleExtensionAvailable(id: string): boolean {
  return useContext(ConsoleExtensionsContext).loadedIds.has(id)
}

export function ConsoleExtensionSlot({
  name,
  context,
  fallback = null,
  className,
}: {
  name: string
  context: Record<string, unknown>
  fallback?: ReactNode
  className?: string
}) {
  const registry = useContext(ConsoleExtensionsContext)
  const contributions = registry.contributions.get(name) ?? []

  if (contributions.length === 0) return fallback
  return (
    <>
      {contributions.map((contribution) => (
        <MountedContribution
          key={contribution.id}
          contribution={contribution}
          context={context}
          className={className}
        />
      ))}
    </>
  )
}

function MountedContribution({
  contribution,
  context,
  className,
}: {
  contribution: ConsoleExtensionSlotContribution
  context: Record<string, unknown>
  className?: string
}) {
  const elementRef = useRef<HTMLDivElement>(null)
  const contextRef = useRef(context)
  const dispatchedContextRef = useRef(context)
  contextRef.current = context

  useEffect(() => {
    const element = elementRef.current
    if (!element) return
    dispatchedContextRef.current = contextRef.current
    const mounted = contribution.mount(element, contextRef.current)
    return () => {
      if (typeof mounted === 'function') mounted()
      else mounted?.dispose()
      element.replaceChildren()
    }
  }, [contribution])

  useEffect(() => {
    if (shallowEqualContext(dispatchedContextRef.current, context)) return
    dispatchedContextRef.current = context
    elementRef.current?.dispatchEvent(
      new CustomEvent('iii:console-extension-context', {
        detail: context,
      }),
    )
  }, [context])

  return <div ref={elementRef} className={className} />
}

function shallowEqualContext(
  previous: Record<string, unknown>,
  next: Record<string, unknown>,
): boolean {
  const previousKeys = Object.keys(previous)
  const nextKeys = Object.keys(next)
  return (
    previousKeys.length === nextKeys.length &&
    previousKeys.every((key) => Object.is(previous[key], next[key]))
  )
}

async function loadExtension(
  client: IiiClient,
  manifestFunction: string,
  registerSlot: ConsoleExtensionHost['registerSlot'],
): Promise<LoadedExtension> {
  const manifest = consoleExtensionManifestSchema.parse(
    await client.trigger(manifestFunction, {}),
  )
  if (manifest.api_version !== CONSOLE_EXTENSION_API_VERSION) {
    throw new Error(
      `${manifest.id} requires console extension API ${manifest.api_version}; host supports ${CONSOLE_EXTENSION_API_VERSION}`,
    )
  }

  const cleanups: Array<() => void> = []
  const styleElements: HTMLStyleElement[] = []
  const fetchAsset = async (
    descriptor: ConsoleExtensionManifest['entry'],
  ): Promise<Uint8Array> => {
    const asset = consoleExtensionAssetSchema.parse(
      await client.trigger(manifest.asset_function, { path: descriptor.path }),
    )
    return verifyExtensionAsset(descriptor, asset)
  }

  for (const descriptor of manifest.styles) {
    const bytes = await fetchAsset(descriptor)
    const style = document.createElement('style')
    style.dataset.consoleExtension = manifest.id
    style.textContent = new TextDecoder().decode(bytes)
    document.head.appendChild(style)
    styleElements.push(style)
  }

  const moduleBytes = await fetchAsset(manifest.entry)
  const moduleBuffer = moduleBytes.buffer.slice(
    moduleBytes.byteOffset,
    moduleBytes.byteOffset + moduleBytes.byteLength,
  ) as ArrayBuffer
  const moduleUrl = URL.createObjectURL(
    new Blob([moduleBuffer], { type: manifest.entry.media_type }),
  )
  try {
    const extension = (await import(
      /* @vite-ignore */ moduleUrl
    )) as ConsoleExtensionModule
    if (typeof extension.activate !== 'function') {
      throw new Error(`${manifest.id} does not export activate(host)`)
    }
    const track = (cleanup: () => void): (() => void) => {
      cleanups.push(cleanup)
      return cleanup
    }
    const host: ConsoleExtensionHost = {
      apiVersion: CONSOLE_EXTENSION_API_VERSION,
      extension: {
        id: manifest.id,
        workerVersion: manifest.worker_version,
      },
      registerSlot: (contribution) => track(registerSlot(contribution)),
      trigger: (functionId, payload) => client.trigger(functionId, payload),
      on: (functionId, handler) => track(client.on(functionId, handler)),
      registerTrigger: (input) => track(client.registerTrigger(input)),
      browserId: client.browserId,
    }
    const activated = await extension.activate(host)
    if (activated) cleanups.push(() => activated.dispose())
  } catch (error) {
    for (const cleanup of cleanups.reverse()) cleanup()
    for (const style of styleElements) style.remove()
    throw error
  } finally {
    URL.revokeObjectURL(moduleUrl)
  }

  return {
    manifestFunction,
    manifest,
    dispose() {
      for (const cleanup of cleanups.reverse()) {
        try {
          cleanup()
        } catch (error) {
          console.warn(
            `[console-extension] ${manifest.id} cleanup failed`,
            error,
          )
        }
      }
      for (const style of styleElements) style.remove()
    },
  }
}
