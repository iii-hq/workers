import type { Theme } from '@/hooks/use-theme'

const MAX_SOURCE_LENGTH = 50_000
let nextDiagramId = 0
let renderQueue = Promise.resolve()

/** Percentage-only SVGs decode as 300px-wide images in Chromium, which also
 * gives the image viewer the wrong natural size. Export an explicit viewport
 * and a small gutter so strokes, arrowheads and shadows are not edge-clipped. */
export function normalizeMermaidSvg(source: string): string {
  const parsed = new DOMParser().parseFromString(source, 'image/svg+xml')
  const svg = parsed.documentElement as unknown as SVGSVGElement
  if (svg.localName !== 'svg' || parsed.querySelector('parsererror')) {
    throw new Error('Invalid Mermaid SVG.')
  }
  const box = (svg.getAttribute('viewBox') ?? '')
    .trim()
    .split(/[\s,]+/)
    .map(Number)
  if (
    box.length !== 4 ||
    !box.every(Number.isFinite) ||
    box[2] <= 0 ||
    box[3] <= 0
  ) {
    throw new Error('Invalid Mermaid viewport.')
  }
  const [x, y, width, height] = box
  let left = x
  let top = y
  let right = x + width
  let bottom = y + height

  // Measure the FINAL sanitized SVG, not the temporary Mermaid render tree.
  // Font metrics, host CSS and browser layout can leave the original viewBox
  // smaller than its nodes. Extra padding around that stale box cannot fix it.
  // A shadow root isolates measurement from the chat's .label/svg/text rules.
  const host = document.createElement('div')
  host.style.cssText =
    'position:fixed;left:0;top:0;width:0;height:0;overflow:hidden;visibility:hidden;pointer-events:none'
  host.setAttribute('aria-hidden', 'true')
  const shadow = host.attachShadow({ mode: 'closed' })
  shadow.append(svg)
  document.body.append(host)
  try {
    const bounds = svg.getBBox()
    if (
      [bounds.x, bounds.y, bounds.width, bounds.height].every(
        Number.isFinite,
      ) &&
      bounds.width > 0 &&
      bounds.height > 0
    ) {
      // Never shrink a valid viewport: some diagrams reserve space for titles,
      // markers or shadows that getBBox does not include.
      left = Math.min(left, bounds.x)
      top = Math.min(top, bounds.y)
      right = Math.max(right, bounds.x + bounds.width)
      bottom = Math.max(bottom, bounds.y + bounds.height)
    }
  } finally {
    host.remove()
  }

  const padding = 16
  const exportWidth = right - left + padding * 2
  const exportHeight = bottom - top + padding * 2
  svg.setAttribute(
    'viewBox',
    `${left - padding} ${top - padding} ${exportWidth} ${exportHeight}`,
  )
  svg.setAttribute('width', String(exportWidth))
  svg.setAttribute('height', String(exportHeight))
  svg.setAttribute('preserveAspectRatio', 'xMidYMid meet')
  // Keep explicit dimensions authoritative when decoded as a standalone image.
  for (const property of ['width', 'height', 'max-width', 'max-height']) {
    svg.style.removeProperty(property)
  }
  return new XMLSerializer().serializeToString(svg)
}

/** Mermaid has global configuration: serialize initialization and rendering so
 * concurrent diagrams and theme changes cannot overwrite each other's config.
 * The heavy library (and its diagram types) stays out of the initial bundle. */
export function renderMermaid(source: string, theme: Theme): Promise<string> {
  if (source.length > MAX_SOURCE_LENGTH) {
    return Promise.reject(new Error('Mermaid source exceeds the size limit.'))
  }

  const task = renderQueue.then(async () => {
    const { default: mermaid } = await import('mermaid')
    await document.fonts?.ready
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      suppressErrorRendering: true,
      theme: theme === 'dark' ? 'dark' : 'default',
      fontFamily: 'sans-serif',
      htmlLabels: false,
      maxTextSize: MAX_SOURCE_LENGTH,
      maxEdges: 500,
      // Diagram directives must not weaken these application-level settings.
      secure: [
        'secure',
        'securityLevel',
        'startOnLoad',
        'suppressErrorRendering',
        'htmlLabels',
        'maxTextSize',
        'maxEdges',
      ],
    })

    // Layout needs an attached DOM node. Keep all temporary SVG/error nodes
    // inside a hidden host and remove it even when parsing or rendering fails.
    const host = document.createElement('div')
    host.style.cssText = 'position:absolute;left:-100000px;visibility:hidden'
    host.setAttribute('aria-hidden', 'true')
    document.body.append(host)
    try {
      const { svg } = await mermaid.render(
        `ade-mermaid-${++nextDiagramId}`,
        source,
        host,
      )
      return normalizeMermaidSvg(svg)
    } finally {
      host.remove()
    }
  })

  // A malformed diagram must not poison the queue for later messages.
  renderQueue = task.then(
    () => undefined,
    () => undefined,
  )
  return task
}
