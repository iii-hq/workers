/**
 * Export plumbing for the preview: turn the currently rendered SVG string
 * into a downloaded .svg, or rasterize it to a .png through an <img> +
 * <canvas> round-trip (2x for sharpness). DOM-dependent by nature — the
 * pure filename logic lives in helpers.ts where it is tested.
 */

const PNG_SCALE = 2

/** Standard blob download: temp anchor, click, revoke. */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}

export function downloadSvg(svg: string, filename: string): void {
  downloadBlob(new Blob([svg], { type: 'image/svg+xml;charset=utf-8' }), filename)
}

/**
 * Mermaid emits `width="100%"` plus an inline `max-width` style; an <img>
 * rasterizing that has no layout to resolve the percentage against. Pin the
 * root to explicit pixel dimensions (from the viewBox) and drop the
 * max-width clamp so the raster comes out at diagram scale.
 */
export function svgWithExplicitSize(
  svg: string,
  width: number,
  height: number,
): string {
  const doc = new DOMParser().parseFromString(svg, 'image/svg+xml')
  if (doc.querySelector('parsererror')) return svg
  const root = doc.documentElement
  root.setAttribute('width', String(width))
  root.setAttribute('height', String(height))
  const style = root.getAttribute('style')
  if (style) {
    root.setAttribute('style', style.replace(/max-width:\s*[^;]+;?/g, ''))
  }
  return new XMLSerializer().serializeToString(root)
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => resolve(img)
    img.onerror = () => reject(new Error('the svg could not be rasterized'))
    img.src = url
  })
}

/**
 * SVG string → PNG blob. `background` (a resolved CSS color, e.g. the
 * preview frame's computed background) is painted under the diagram so a
 * dark-theme render doesn't come out as light strokes on transparency;
 * omit it for a transparent png.
 */
export async function rasterizeSvg(
  svg: string,
  width: number,
  height: number,
  background?: string,
): Promise<Blob> {
  const sized = svgWithExplicitSize(svg, width, height)
  const url = URL.createObjectURL(
    new Blob([sized], { type: 'image/svg+xml;charset=utf-8' }),
  )
  try {
    const img = await loadImage(url)
    const canvas = document.createElement('canvas')
    canvas.width = Math.max(1, Math.round(width * PNG_SCALE))
    canvas.height = Math.max(1, Math.round(height * PNG_SCALE))
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('canvas 2d context unavailable')
    if (background && background !== 'transparent') {
      ctx.fillStyle = background
      ctx.fillRect(0, 0, canvas.width, canvas.height)
    }
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height)
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob(
        (blob) =>
          blob ? resolve(blob) : reject(new Error('png encoding failed')),
        'image/png',
      )
    })
  } finally {
    URL.revokeObjectURL(url)
  }
}

export async function downloadSvgAsPng(
  svg: string,
  width: number,
  height: number,
  filename: string,
  background?: string,
): Promise<void> {
  downloadBlob(await rasterizeSvg(svg, width, height, background), filename)
}
