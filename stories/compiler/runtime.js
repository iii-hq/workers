// Preview runtime for the stories worker.
//
// Framework-agnostic on purpose: the generated entry passes in the PROJECT's
// React and react-dom/client so a story renders with the same React instance
// its components were written against. Implements the CSF3 subset the worker
// supports (meta.{title,component,args,argTypes,parameters,decorators,tags},
// story.{name,args,argTypes,parameters,decorators,render,tags}, plus the
// preview file's {decorators,parameters,globalTypes,initialGlobals}). `play`
// functions, MDX and addons are ignored.
//
// Three duties:
//   manifest()  — the components/states catalogue (evaluated in Node too)
//   render()    — mount one state, with args/globals overrides
//   capture()   — DOM + React tree snapshot for stories::tree / stories::diff

const registry = (globalThis.__storiesRegistry ||= {})

const STYLE_PROPS = [
  'display', 'position', 'top', 'right', 'bottom', 'left', 'z-index', 'box-sizing',
  'width', 'height', 'min-width', 'max-width', 'min-height', 'max-height',
  'margin-top', 'margin-right', 'margin-bottom', 'margin-left',
  'padding-top', 'padding-right', 'padding-bottom', 'padding-left',
  'border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width',
  'border-top-style', 'border-top-color', 'border-top-left-radius', 'border-top-right-radius',
  'border-bottom-left-radius', 'border-bottom-right-radius',
  'color', 'background-color', 'background-image', 'opacity', 'visibility',
  'overflow-x', 'overflow-y', 'font-family', 'font-size', 'font-weight', 'font-style',
  'line-height', 'letter-spacing', 'text-align', 'text-decoration-line', 'text-transform',
  'white-space', 'text-overflow', 'vertical-align',
  'flex-direction', 'flex-wrap', 'flex-grow', 'flex-shrink', 'flex-basis',
  'justify-content', 'align-items', 'align-self', 'gap', 'row-gap', 'column-gap',
  'grid-template-columns', 'grid-template-rows', 'transform', 'box-shadow',
  'outline-width', 'outline-style', 'outline-color', 'cursor', 'pointer-events', 'object-fit',
]

const ATTRS = new Set([
  'id', 'role', 'type', 'href', 'src', 'alt', 'value', 'placeholder', 'name', 'title',
  'disabled', 'checked', 'selected', 'tabindex', 'for', 'open', 'hidden', 'lang', 'dir',
])

/* ── ids and names (Storybook's algorithms) ───────────────────────────── */

export function sanitize(text) {
  return String(text)
    .toLowerCase()
    .replace(/[ ’–—―′¿'`~!@#$%^&*()_|+\-=?;:'",.<>{}[\]\\/]/gi, '-')
    .replace(/-+/g, '-')
    .replace(/^-+/, '')
    .replace(/-+$/, '')
}

export function toId(title, exportName) {
  return `${sanitize(title)}--${sanitize(exportName)}`
}

export function storyNameFromExport(key) {
  return String(key)
    .replace(/([a-z\d])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .replace(/[_\-]+/g, ' ')
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((word) => word[0].toUpperCase() + word.slice(1))
    .join(' ')
}

function matches(pattern, name) {
  if (!pattern) return false
  if (Array.isArray(pattern)) return pattern.includes(name)
  if (pattern instanceof RegExp) return pattern.test(name)
  return false
}

function isStoryExport(key, value, meta) {
  if (key === 'default' || key === '__namedExportsOrder') return false
  if (meta.includeStories && !matches(meta.includeStories, key)) return false
  if (matches(meta.excludeStories, key)) return false
  return (value && typeof value === 'object') || typeof value === 'function'
}

/* ── serialization ────────────────────────────────────────────────────── */

function elementName(React, element) {
  const type = element.type
  if (typeof type === 'string') return type
  if (typeof type === 'function') return type.displayName || type.name || 'Component'
  if (type && typeof type === 'object') return type.displayName || type.render?.name || type.type?.name || 'Component'
  if (type === React.Fragment) return 'Fragment'
  return 'Element'
}

export function serialize(React, value, depth = 0) {
  if (value === undefined) return { $undefined: true }
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value
  if (typeof value === 'number') return Number.isFinite(value) ? value : { $number: String(value) }
  if (typeof value === 'function') return { $fn: value.mockName || value.name || 'fn' }
  if (typeof value === 'bigint') return { $bigint: value.toString() }
  if (typeof value === 'symbol') return { $symbol: value.description ?? '' }
  if (React?.isValidElement?.(value)) return { $element: elementName(React, value) }
  if (value instanceof Date) return { $date: value.toISOString() }
  if (value instanceof RegExp) return { $regexp: value.toString() }
  if (depth > 6) return { $truncated: true }
  if (Array.isArray(value)) return value.slice(0, 200).map((item) => serialize(React, item, depth + 1))
  if (typeof value === 'object') {
    const out = {}
    for (const [key, item] of Object.entries(value).slice(0, 200)) out[key] = serialize(React, item, depth + 1)
    return out
  }
  return { $type: typeof value }
}

function controlType(raw) {
  switch (raw) {
    case 'select':
    case 'radio':
    case 'inline-radio':
    case 'check':
    case 'inline-check':
    case 'multi-select':
      return 'select'
    case 'boolean':
      return 'boolean'
    case 'number':
    case 'range':
      return 'number'
    case 'object':
      return 'object'
    case 'text':
    case 'color':
    case 'date':
    case 'file':
      return 'text'
    default:
      return null
  }
}

export function inferControl(React, name, argType, value) {
  const raw = argType?.control
  const explicit = typeof raw === 'string' ? raw : raw && typeof raw === 'object' ? raw.type : null
  const options = argType?.options ?? (raw && typeof raw === 'object' ? raw.options : undefined)
  const serialized = serialize(React, value)
  const base = { name, default: serialized }
  if (argType?.description) base.description = String(argType.description)
  if (raw === false) return { ...base, type: 'readonly' }
  const fromExplicit = explicit ? controlType(explicit) : null
  if (fromExplicit) return { ...base, type: fromExplicit, ...(options ? { options: serialize(React, options) } : {}) }
  if (options) return { ...base, type: 'select', options: serialize(React, options) }
  const declared = argType?.type?.name ?? argType?.type
  if (declared === 'boolean') return { ...base, type: 'boolean' }
  if (declared === 'number') return { ...base, type: 'number' }
  if (declared === 'string') return { ...base, type: 'text' }
  if (typeof value === 'function') return { ...base, type: 'function' }
  if (React?.isValidElement?.(value)) return { ...base, type: 'element' }
  if (typeof value === 'boolean') return { ...base, type: 'boolean' }
  if (typeof value === 'number') return { ...base, type: 'number' }
  if (typeof value === 'string') return { ...base, type: 'text' }
  if (value && typeof value === 'object') return { ...base, type: 'object' }
  return { ...base, type: 'text' }
}

/* ── manifest ─────────────────────────────────────────────────────────── */

function componentName(meta, file) {
  const component = meta.component
  if (typeof component === 'function') return component.displayName || component.name || null
  if (component && typeof component === 'object') return component.displayName || component.render?.name || null
  if (meta.title) return String(meta.title).split('/').pop()
  return file.split('/').pop().replace(/\.stories\.[jt]sx?$/, '')
}

export function buildManifest({ React, stories, preview, file }) {
  const meta = stories.default ?? {}
  const title = meta.title ? String(meta.title) : file.replace(/\.stories\.[jt]sx?$/, '')
  const segments = title.split('/').filter(Boolean)
  const group = segments.length > 1 ? segments.slice(0, -1).join('/') : ''
  const states = []
  for (const [key, value] of Object.entries(stories)) {
    if (!isStoryExport(key, value, meta)) continue
    const story = typeof value === 'function' ? { ...value, render: value } : value
    const name = story.name ?? story.storyName ?? storyNameFromExport(key)
    const args = { ...(meta.args ?? {}), ...(story.args ?? {}) }
    const argTypes = { ...(meta.argTypes ?? {}), ...(story.argTypes ?? {}) }
    const names = new Set([...Object.keys(argTypes), ...Object.keys(args)])
    const controls = [...names].map((argName) => inferControl(React, argName, argTypes[argName], args[argName]))
    states.push({
      id: toId(title, key),
      name,
      export_name: key,
      tags: [...(story.tags ?? [])],
      args: serialize(React, args),
      arg_types: serialize(React, argTypes),
      parameters: serialize(React, { ...(story.parameters ?? {}) }),
      controls,
      has_render: typeof story.render === 'function' || typeof meta.render === 'function',
      decorators: (story.decorators ?? []).length,
    })
  }
  return {
    id: sanitize(title),
    title,
    group,
    component: componentName(meta, file),
    tags: [...(meta.tags ?? [])],
    parameters: serialize(React, meta.parameters ?? {}),
    globals: serialize(React, { ...(preview?.initialGlobals ?? preview?.globals ?? {}) }),
    global_types: serialize(React, preview?.globalTypes ?? {}),
    decorators: (meta.decorators ?? []).length + (preview?.decorators ?? []).length,
    states,
  }
}

/* ── deterministic mode ───────────────────────────────────────────────── */

const FROZEN_CSS = `
*, *::before, *::after { animation: none !important; transition: none !important; caret-color: transparent !important; scroll-behavior: auto !important; }
html { scrollbar-width: none !important; }
::-webkit-scrollbar { display: none !important; }
`

let frozen = false
export function freeze() {
  if (frozen) return
  frozen = true
  const fixed = Date.UTC(2026, 0, 1, 12, 0, 0)
  const RealDate = Date
  class FrozenDate extends RealDate {
    constructor(...args) {
      if (args.length === 0) super(fixed)
      else super(...args)
    }
    static now() {
      return fixed
    }
  }
  globalThis.Date = FrozenDate
  let seed = 0x2f6e2b1
  Math.random = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0
    return seed / 4294967296
  }
  const RealIntl = globalThis.Intl
  if (RealIntl) {
    const wrap = (Ctor) =>
      new Proxy(Ctor, {
        construct(target, [locales, options]) {
          return new target(locales ?? 'en-US', { timeZone: 'UTC', ...(options ?? {}) })
        },
        apply(target, thisArg, [locales, options]) {
          return target.call(thisArg, locales ?? 'en-US', { timeZone: 'UTC', ...(options ?? {}) })
        },
      })
    globalThis.Intl = { ...RealIntl, DateTimeFormat: wrap(RealIntl.DateTimeFormat) }
    RealDate.prototype.getTimezoneOffset = () => 0
  }
  if (typeof document !== 'undefined') {
    const style = document.createElement('style')
    style.id = 'stories-frozen'
    style.textContent = FROZEN_CSS
    document.head.appendChild(style)
  }
}

/* ── capture ──────────────────────────────────────────────────────────── */

function fiberOf(node) {
  const key = Object.keys(node).find((name) => name.startsWith('__reactFiber$'))
  return key ? node[key] : null
}

function fiberName(fiber) {
  const type = fiber?.type
  if (typeof type === 'function') return type.displayName || type.name || null
  if (type && typeof type === 'object') return type.displayName || type.render?.name || type.type?.displayName || type.type?.name || null
  return null
}

function ownerComponent(node) {
  let fiber = fiberOf(node)
  fiber = fiber?.return
  while (fiber) {
    const name = fiberName(fiber)
    if (name) return name
    fiber = fiber.return
  }
  return null
}

function round(value) {
  return Math.round(value * 2) / 2
}

function serializeElement(el, origin, computed) {
  const rect = el.getBoundingClientRect()
  const style = computed(el)
  const styles = {}
  for (const prop of STYLE_PROPS) styles[prop] = style.getPropertyValue(prop)
  const attrs = {}
  for (const attr of el.attributes) {
    if (attr.name === 'class' || attr.name === 'style') continue
    if (ATTRS.has(attr.name) || attr.name.startsWith('aria-') || attr.name.startsWith('data-')) attrs[attr.name] = attr.value
  }
  let text = ''
  for (const child of el.childNodes) if (child.nodeType === 3) text += child.nodeValue
  text = text.replace(/\s+/g, ' ').trim().slice(0, 200)
  const children = []
  for (const child of el.children) children.push(serializeElement(child, origin, computed))
  const out = {
    tag: el.tagName.toLowerCase(),
    box: { x: round(rect.left - origin.x), y: round(rect.top - origin.y), w: round(rect.width), h: round(rect.height) },
    styles,
    children,
  }
  const classes = [...el.classList].sort()
  if (classes.length) out.classes = classes
  if (Object.keys(attrs).length) out.attrs = attrs
  if (text) out.text = text
  const component = ownerComponent(el)
  if (component) out.component = component
  return out
}

function reactTree(React, container) {
  const key = Object.keys(container).find((name) => name.startsWith('__reactContainer$'))
  let root = key ? container[key] : container._reactRootContainer?._internalRoot?.current
  if (!root) return null
  // The container keeps the HostRoot fiber minted at createRoot(); after a
  // commit the live tree hangs off the FiberRoot's `current`, which may be
  // that fiber's alternate.
  root = root.stateNode?.current ?? root
  const internal = new Set(['Boundary', 'Story'])
  const walk = (fiber) => {
    const nodes = []
    for (let child = fiber.child; child; child = child.sibling) {
      const name = fiberName(child)
      const type = child.type
      if ((name && internal.has(name)) || (type === 'div' && child.memoizedProps?.id === 'story-content')) {
        nodes.push(...walk(child))
      } else if (name) {
        const props = {}
        for (const [prop, value] of Object.entries(child.memoizedProps ?? {})) {
          if (prop === 'children') continue
          props[prop] = serialize(React, value, 2)
        }
        nodes.push({ name, props, children: walk(child) })
      } else if (typeof type === 'string') {
        nodes.push({ tag: type, children: walk(child) })
      } else if (typeof child.memoizedProps === 'string' || typeof child.memoizedProps === 'number') {
        const text = String(child.memoizedProps).replace(/\s+/g, ' ').trim()
        if (text) nodes.push({ text: text.slice(0, 200) })
      } else {
        nodes.push(...walk(child))
      }
    }
    return nodes
  }
  return walk(root)
}

function contentBox(content) {
  const children = [...content.children]
  const boxes = (children.length ? children : [content]).map((el) => el.getBoundingClientRect())
  const left = Math.min(...boxes.map((b) => b.left))
  const top = Math.min(...boxes.map((b) => b.top))
  const right = Math.max(...boxes.map((b) => b.right))
  const bottom = Math.max(...boxes.map((b) => b.bottom))
  return { x: Math.max(0, Math.floor(left)), y: Math.max(0, Math.floor(top)), w: Math.ceil(right - left), h: Math.ceil(bottom - top) }
}

/* ── the per-file api ─────────────────────────────────────────────────── */

const LAYOUT_CSS = `
html, body { margin: 0; }
#story-root[data-layout="padded"] > #story-content { padding: 1rem; }
#story-root[data-layout="centered"] { min-height: 100vh; display: flex; align-items: center; justify-content: center; }
#story-root[data-layout="fullscreen"] > #story-content { padding: 0; }
#story-error { font: 12px/1.5 ui-monospace, monospace; white-space: pre-wrap; color: #b00020; padding: 1rem; }
`

function createApi({ React, ReactDOMClient, stories, preview, file }) {
  const manifest = () => buildManifest({ React, stories, preview, file })
  let root = null
  let container = null
  let content = null
  let current = null

  function post(message) {
    if (typeof window === 'undefined' || window.parent === window) return
    try {
      window.parent.postMessage({ source: 'iii-stories', file, ...message }, '*')
    } catch {}
  }

  function resolveState(story) {
    const meta = stories.default ?? {}
    const title = meta.title ? String(meta.title) : file.replace(/\.stories\.[jt]sx?$/, '')
    for (const [key, value] of Object.entries(stories)) {
      if (!isStoryExport(key, value, meta)) continue
      const object = typeof value === 'function' ? { ...value, render: value } : value
      const name = object.name ?? object.storyName ?? storyNameFromExport(key)
      if (story === key || story === name || story === toId(title, key) || story === undefined) {
        return { key, story: object, meta, title, id: toId(title, key), name }
      }
    }
    return null
  }

  class Boundary extends React.Component {
    constructor(props) {
      super(props)
      this.state = { error: null }
    }
    static getDerivedStateFromError(error) {
      return { error }
    }
    componentDidCatch(error) {
      post({ type: 'stories:error', message: String(error?.stack ?? error) })
    }
    render() {
      if (this.state.error) {
        return React.createElement('pre', { id: 'story-error' }, String(this.state.error?.stack ?? this.state.error))
      }
      return this.props.children
    }
  }

  function ensureMounted() {
    if (root) return
    container = document.getElementById('story-root')
    if (!container) {
      container = document.createElement('div')
      container.id = 'story-root'
      document.body.appendChild(container)
    }
    if (!document.getElementById('stories-layout')) {
      const style = document.createElement('style')
      style.id = 'stories-layout'
      style.textContent = LAYOUT_CSS
      document.head.appendChild(style)
    }
    root = ReactDOMClient.createRoot(container)
  }

  async function settle() {
    try {
      await document.fonts?.ready
    } catch {}
    await Promise.all([...document.images].filter((img) => !img.complete).map((img) => img.decode().catch(() => {})))
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  }

  async function render(options = {}) {
    const { story, args: overrides = {}, globals: globalOverrides = {}, deterministic = false } = options
    ensureMounted()
    document.documentElement.dataset.storiesReady = ''
    const resolved = resolveState(story)
    if (!resolved) {
      const message = `unknown story "${story}" in ${file}`
      root.render(React.createElement('pre', { id: 'story-error' }, message))
      post({ type: 'stories:error', message })
      return { ok: false, error: message }
    }
    if (deterministic) freeze()
    const { key, story: object, meta, title, id, name } = resolved
    const args = { ...(meta.args ?? {}), ...(object.args ?? {}), ...overrides }
    const globals = { ...(preview?.initialGlobals ?? preview?.globals ?? {}), ...globalOverrides }
    const parameters = { ...(preview?.parameters ?? {}), ...(meta.parameters ?? {}), ...(object.parameters ?? {}) }
    const context = {
      id, name, title, kind: title, story: name, component: meta.component,
      args, initialArgs: args, argTypes: { ...(meta.argTypes ?? {}), ...(object.argTypes ?? {}) },
      globals, parameters, viewMode: 'story', hooks: {}, loaded: {}, abortSignal: new AbortController().signal,
      canvasElement: container, tags: [...(meta.tags ?? []), ...(object.tags ?? [])],
    }
    const renderFn = object.render ?? meta.render ?? ((props) => React.createElement(meta.component, props))
    const base = () => renderFn(args, context)
    Object.defineProperty(base, 'name', { value: '' })
    const decorators = [...(object.decorators ?? []), ...(meta.decorators ?? []), ...(preview?.decorators ?? [])]
    const composed = decorators.reduce((inner, decorator) => () => decorator(inner, context), base)
    const Story = () => composed()
    Story.displayName = 'Story'
    current = { id, key, args, globals, parameters }
    container.dataset.layout = parameters.layout ?? 'padded'
    root.render(
      React.createElement(
        Boundary,
        { key: `${id}:${JSON.stringify(serialize(React, args))}:${JSON.stringify(globals)}` },
        React.createElement('div', { id: 'story-content' }, React.createElement(Story)),
      ),
    )
    await settle()
    content = document.getElementById('story-content')
    const box = content ? contentBox(content) : { x: 0, y: 0, w: 0, h: 0 }
    document.documentElement.dataset.storiesReady = '1'
    const result = { ok: true, id, height: document.documentElement.scrollHeight, box }
    post({ type: 'stories:rendered', ...result })
    return result
  }

  function capture() {
    const target = document.getElementById('story-content')
    if (!target) return { error: 'nothing rendered' }
    const origin = contentBox(target)
    const computed = (el) => getComputedStyle(el)
    const dom = [...target.children].map((el) => serializeElement(el, origin, computed))
    return {
      story: current?.id ?? null,
      args: current ? serialize(React, current.args) : null,
      globals: current?.globals ?? null,
      box: origin,
      dom,
      react: reactTree(React, container),
      html: target.innerHTML,
      actions: [...(globalThis.__storiesActions ?? [])],
    }
  }

  function autostart() {
    const params = new URLSearchParams(location.search)
    const story = params.get('story')
    const decode = (name) => {
      const raw = params.get(name)
      if (!raw) return {}
      try {
        return JSON.parse(atob(raw.replace(/-/g, '+').replace(/_/g, '/')))
      } catch {
        return {}
      }
    }
    window.addEventListener('message', (event) => {
      const data = event.data
      if (!data || data.source !== 'iii-stories-host') return
      if (data.type === 'stories:render') render(data).catch(() => {})
      if (data.type === 'stories:capture') post({ type: 'stories:captured', requestId: data.requestId, capture: capture() })
    })
    post({ type: 'stories:ready', manifest: manifest() })
    if (story) {
      render({ story, args: decode('args'), globals: decode('globals'), deterministic: params.get('deterministic') === '1' }).catch(
        (error) => post({ type: 'stories:error', message: String(error?.stack ?? error) }),
      )
    }
  }

  return { file, manifest, render, capture, autostart, actions: () => [...(globalThis.__storiesActions ?? [])] }
}

export function boot(options) {
  const api = createApi(options)
  registry[options.file] = api
  if (typeof window !== 'undefined' && typeof document !== 'undefined' && document.getElementById('story-root')) {
    window.__stories = api
    api.autostart()
  }
  return api
}
