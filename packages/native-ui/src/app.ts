/**
 * The app: the page table `defineApp` declares (sent in `ready`), page
 * instances the host mounts and unmounts, and the events the host sends to
 * nodes (design §5.3, §5.6, §10.2, §10.4).
 */

import { catchError, createComponent, createContext, createRenderEffect, createRoot, type JSX } from 'solid-js'
import { CATALOG_VERSION, FEATURES, IN, OP, ROOT_PAGE } from './generated/catalog'
import { appCatalog, clearPage, ElementNode, hasPageRoot, nodeById, setPageRoot, warnOnce } from './runtime/tree'
import { isVmStop, onEntry, push, reportError, run } from './runtime/wire'
import type { JsonObject } from './types'
import { SDK_VERSION } from './version'

/** A mounted page instance. */
export interface PageRef {
  readonly inst: number
  readonly name: string
  readonly params: Readonly<JsonObject>
}

export type PageComponent = (props: { page: PageRef; params: JsonObject }) => JSX.Element

export interface PageOptions {
  component: PageComponent
  /** A modal card: the host draws an X instead of the chevron and hides the app toolbar. */
  modal?: boolean
  /** The card takes the whole height (the app toolbar hides). */
  fullScreen?: boolean
}

export interface AppDefinition {
  /**
   * The lowest catalogue version the app needs (default 1), sent in `ready`:
   * an iii app with an older catalogue shows "Needs a newer iii app" instead
   * of running it. Declare 2 to use `Markdown`, `Lanes`, `hue`, `dragGroup`
   * or `listFill` (the SDK warns when a newer type or prop is used without
   * it), and give the worker's descriptor the same (`NativeUi::catalog(2)`).
   */
  catalog?: number
  pages: Record<string, PageComponent | PageOptions>
}

interface PageDef {
  component: PageComponent
  modal: boolean
  fullScreen: boolean
}

/** The pages `defineApp` declared, by name. */
export const declaredPages = new Map<string, PageDef>()

export const PageContext = createContext<PageRef>()

/**
 * Declares the app's pages and tells the host the bundle is ready. Call it
 * once, at the top level of the bundle; `root` is required.
 */
export function defineApp(app: AppDefinition): void {
  if (declaredPages.size) throw new Error('defineApp was already called')
  const catalog = app?.catalog ?? 1
  if (!Number.isInteger(catalog) || catalog < 1 || catalog > CATALOG_VERSION) {
    throw new Error(`defineApp: catalog is a version from 1 to ${CATALOG_VERSION} (this SDK's), got ${catalog}`)
  }
  const entries = Object.entries(app?.pages ?? {})
  if (!entries.some(([name]) => name === ROOT_PAGE)) throw new Error('defineApp: the pages need a "root" page')
  for (const [name, page] of entries) {
    const def = typeof page === 'function' ? { component: page } : page
    if (!name || typeof def?.component !== 'function') throw new Error(`defineApp: page "${name}" has no component`)
    declaredPages.set(name, { component: def.component, modal: !!def.modal, fullScreen: !!def.fullScreen })
  }
  appCatalog.version = catalog
  push([
    OP.ready,
    {
      sdk: SDK_VERSION,
      catalog,
      features: [...FEATURES],
      pages: [...declaredPages].map(([name, d]) => ({ name, modal: d.modal, fullScreen: d.fullScreen })),
    },
  ])
}

const instances = new Map<number, { dispose: () => void; ref: PageRef }>()

/** The single node a page's component rendered (functions are read in a tracking scope). */
function rootNode(value: unknown): ElementNode | null {
  while (typeof value === 'function') value = value()
  if (Array.isArray(value)) {
    const nodes = value.map(rootNode).filter((n): n is ElementNode => n !== null)
    if (nodes.length > 1) warnOnce('a page renders one root node; the others are ignored')
    return nodes[0] ?? null
  }
  if (value instanceof ElementNode) return value
  if (value !== null && value !== undefined && typeof value !== 'boolean') warnOnce('a page must render a component')
  return null
}

function mount(inst: number, name: string, params: JsonObject): void {
  if (instances.has(inst)) unmount(inst)
  const def = declaredPages.get(name)
  if (!def) {
    reportError(new Error(`the host mounted page "${name}", which defineApp did not declare`))
    return
  }
  const ref: PageRef = Object.freeze({ inst, name, params: Object.freeze({ ...params }) })
  createRoot((dispose) => {
    instances.set(inst, { dispose, ref })
    catchError(
      () => {
        const content = createComponent(PageContext.Provider, {
          value: ref,
          get children() {
            return createComponent(def.component, { page: ref, params: ref.params as JsonObject })
          },
        })
        createRenderEffect(() => setPageRoot(inst, rootNode(content)))
      },
      // A page that never rendered is fatal (the failure screen with
      // Restart); a later error in one of its effects is not. Solid runs
      // this handler after the render, so the root tells which one it is.
      (err) => {
        if (isVmStop(err)) throw err
        reportError(err, { fatal: !hasPageRoot(inst) })
      },
    )
  })
}

function unmount(inst: number): void {
  const instance = instances.get(inst)
  if (!instance) return
  instances.delete(inst)
  instance.dispose()
  clearPage(inst)
}

onEntry(IN.mount, ([, page, name, params]) => {
  mount(page as number, String(name), (params && typeof params === 'object' ? params : {}) as JsonObject)
})

onEntry(IN.unmount, ([, page]) => unmount(page as number))

onEntry(IN.event, ([, id, event, args]) => {
  const node = nodeById(id)
  if (!node) return // destroyed meanwhile (design §5.6)
  const list = Array.isArray(args) ? args : []
  const handler = node.handlers?.get(event as number)
  if (handler) run(() => handler(...list))
  node.afterEvent?.(event as number, list)
})

onEntry(IN.itemEvent, ([, listId, key, id, event, args]) => {
  const list = nodeById(listId)
  const handler = nodeById(id)?.handlers?.get(event as number)
  if (!list?.list || !handler) return
  const item = list.list.get(key)
  if (item === undefined) return
  const rest = Array.isArray(args) ? args : []
  run(() => handler(item, ...rest))
})

onEntry(IN.text, ([, id, count, text]) => {
  const node = nodeById(id)
  if (node?.onText) run(() => node.onText?.(count as number, String(text ?? '')))
})
