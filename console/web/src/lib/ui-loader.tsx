/**
 * The browser loader for injectable UI — Vite HMR semantics without Vite
 * (iii/tech-specs/2026-07-17-injectable-ui/hot-reload.md).
 *
 * Boot is one step: register a `console:assets` trigger whose function_id
 * is this tab's own handler. The console worker answers with a `sync` push
 * (the subscription IS the seed) and follows with incremental `set`/`delete`
 * pushes. The loader diffs every event against loaded state — new/changed
 * hash ⇒ dispose + cache-busted re-import + `setup(host)`; missing path ⇒
 * dispose. Styles are `<link>` swaps; scripts are ES modules.
 */

import { ErrorBoundary } from '@/components/ui/ErrorBoundary'
import type { IiiClient } from '@/lib/iii-client'
import {
  registerExtConfigForm,
  registerExtPage,
  registerExtRenderer,
  registerExtSessionChip,
} from '@/lib/ui-slots'
import type {
  ConfigFormProps,
  ConsoleApi,
  Host,
  SessionChipProps,
  SetupFn,
  UiAssetKind,
  UiAssetRef,
  UiAssetsPush,
} from '@/types/injectable-ui'

/** The per-tab push handler id; `client.on` namespaces it `::<browserId>`.
 *  The `iii::` prefix keeps rebuild-loop pushes span-suppressed. */
export const UI_ASSETS_FN = 'iii::console::ui-assets'

interface LoadedScript {
  kind: 'script'
  path: string
  hash: string
  /** Every host registration + any setup() teardown, run LIFO on dispose. */
  cleanups: Array<() => void>
}

interface LoadedStyle {
  kind: 'style'
  path: string
  hash: string
  link: HTMLLinkElement
}

type Loaded = LoadedScript | LoadedStyle

/**
 * The scope wrapper every injected render mounts inside: `data-iii-ui`
 * carries the first segment of the script's path (worker CSS compiles
 * selector-scoped under it); `display: contents` keeps it out of layout.
 * Render-time crashes degrade to a chip naming the script, never a white
 * screen.
 */
export function ScopedExtension({
  scope,
  path,
  children,
}: {
  scope: string
  path: string
  children: React.ReactNode
}) {
  return (
    <div data-iii-ui={scope} style={{ display: 'contents' }}>
      <ErrorBoundary
        fallback={(error) => <ExtErrorChip path={path} error={error} />}
      >
        {children}
      </ErrorBoundary>
    </div>
  )
}

export function ExtErrorChip({ path, error }: { path: string; error: Error }) {
  return (
    <span
      className="inline-flex items-center gap-1 border border-alert px-2 py-0.5 font-mono text-[11px] text-alert"
      title={error.message}
    >
      extension crashed · {path}
    </span>
  )
}

function makeHost(
  api: ConsoleApi,
  path: string,
  cleanups: Array<() => void>,
): Host {
  const scope = path.split('/')[0]
  const track = (off: () => void): (() => void) => {
    cleanups.push(off)
    return off
  }
  return {
    iii: api.iii,
    components: api.components,
    useTheme: api.useTheme,
    path,
    pages: {
      register(page) {
        const Body = page.render
        return track(
          registerExtPage({
            ...page,
            scope,
            path,
            render: (renderProps) => (
              <ScopedExtension scope={scope} path={path}>
                <Body {...renderProps} />
              </ScopedExtension>
            ),
          }),
        )
      },
    },
    functionTriggers: {
      register(renderer) {
        return track(registerExtRenderer({ renderer, scope, path }))
      },
    },
    configForms: {
      register(configurationId, component) {
        const Form = component
        return track(
          registerExtConfigForm({
            configurationId,
            scope,
            path,
            component: (props: ConfigFormProps) => (
              <ScopedExtension scope={scope} path={path}>
                <Form {...props} />
              </ScopedExtension>
            ),
          }),
        )
      },
    },
    chat: {
      registerSessionChip(chip) {
        const Chip = chip.render
        return track(
          registerExtSessionChip({
            ...chip,
            scope,
            path,
            render: (props: SessionChipProps) => (
              <ScopedExtension scope={scope} path={path}>
                <Chip {...props} />
              </ScopedExtension>
            ),
          }),
        )
      },
    },
  }
}

/**
 * Start the loader against the shared client. Returns a teardown that
 * unsubscribes and disposes every loaded asset (tests; the real tab never
 * calls it).
 */
export function startUiLoader(client: IiiClient, api: ConsoleApi): () => void {
  const loaded = new Map<string, Loaded>()
  // Asset URLs resolve against the DOCUMENT base (the console supports
  // arbitrary subpath mounting); the loader itself lives in a Vite chunk
  // under assets/, so module-relative resolution would be wrong.
  const base = new URL('.', document.baseURI)

  function dispose(path: string) {
    const entry = loaded.get(path)
    if (!entry) return
    loaded.delete(path)
    if (entry.kind === 'style') {
      entry.link.remove()
      return
    }
    for (const cleanup of [...entry.cleanups].reverse()) {
      try {
        cleanup()
      } catch (err) {
        console.error(`[iii-ui] cleanup for ${path} threw`, err)
      }
    }
  }

  function assetUrl(path: string, hash: string): string {
    return new URL(`ui/${path}?v=${hash}`, base).href
  }

  async function applyScript(path: string, hash: string) {
    dispose(path)
    const cleanups: Array<() => void> = []
    try {
      const mod = (await import(/* @vite-ignore */ assetUrl(path, hash))) as {
        default?: SetupFn
      }
      if (typeof mod.default !== 'function') {
        throw new Error('no default setup() export')
      }
      const host = makeHost(api, path, cleanups)
      const teardown = await mod.default(host)
      if (typeof teardown === 'function') cleanups.push(teardown)
      loaded.set(path, { kind: 'script', path, hash, cleanups })
    } catch (err) {
      // Non-fatal: the previous version was already disposed, so this
      // script's contributions simply drop out until the next good version.
      for (const cleanup of [...cleanups].reverse()) {
        try {
          cleanup()
        } catch {
          /* already failing */
        }
      }
      console.error(`[iii-ui] failed to load ${path}@${hash}`, err)
    }
  }

  function applyStyle(path: string, hash: string) {
    const previous = loaded.get(path)
    const link = document.createElement('link')
    link.rel = 'stylesheet'
    link.dataset.iiiUiAsset = path
    link.href = assetUrl(path, hash)
    if (previous?.kind === 'style') {
      // Vite's link-swap: remove the old sheet only once the new one is in,
      // so there is no flash of unstyled extension.
      loaded.delete(path)
      link.addEventListener('load', () => previous.link.remove())
      link.addEventListener('error', () => {
        console.error(`[iii-ui] failed to load stylesheet ${path}@${hash}`)
        link.remove()
      })
      previous.link.after(link)
    } else {
      document.head.appendChild(link)
    }
    loaded.set(path, { kind: 'style', path, hash, link })
  }

  async function applySet(path: string, kind: UiAssetKind, hash: string) {
    const current = loaded.get(path)
    if (current?.hash === hash) return // dedupe (replay, reconnect sync)
    if (kind === 'style') {
      applyStyle(path, hash)
    } else {
      await applyScript(path, hash)
    }
  }

  async function applySync(assets: UiAssetRef[]) {
    const advertised = new Set(assets.map((a) => a.path))
    for (const path of [...loaded.keys()]) {
      if (!advertised.has(path)) dispose(path)
    }
    // Styles before scripts, so a script's first paint has its sheet.
    const ordered = [...assets].sort((a, b) =>
      a.kind === b.kind ? 0 : a.kind === 'style' ? -1 : 1,
    )
    for (const asset of ordered) {
      await applySet(asset.path, asset.kind, asset.hash)
    }
  }

  // Pushes can arrive while a previous import is still in flight; a simple
  // promise chain keeps application ordered per tab.
  let queue: Promise<void> = Promise.resolve()
  const enqueue = (work: () => Promise<void> | void) => {
    queue = queue.then(work).catch((err) => {
      console.error('[iii-ui] loader event failed', err)
    })
  }

  const offHandler = client.on<UiAssetsPush>(UI_ASSETS_FN, (payload) => {
    enqueue(() => {
      if (!payload || typeof payload !== 'object') return
      switch (payload.event) {
        case 'sync':
          return applySync(Array.isArray(payload.assets) ? payload.assets : [])
        case 'set':
          return applySet(payload.path, payload.kind, payload.hash)
        case 'delete':
          return void dispose(payload.path)
        default:
          return
      }
    })
  })

  const offTrigger = client.registerTrigger({
    type: 'console:assets',
    function_id: `${UI_ASSETS_FN}::${client.browserId}`,
    config: {},
  })

  return () => {
    offTrigger()
    offHandler()
    for (const path of [...loaded.keys()]) dispose(path)
  }
}
