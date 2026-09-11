export type DiagramKind = 'network' | 'matrix' | 'radar' | 'loop' | 'ladder'

export interface DiagramNode {
  label: string
  text?: string
  x?: number
  y?: number
  value?: number
  hub?: boolean
}

export interface DiagramEdge {
  from: string
  to: string
}

export interface DiagramAxis {
  label: string
  low?: string
  high?: string
}

export interface DiagramSpec {
  kind: DiagramKind
  nodes: DiagramNode[]
  edges?: DiagramEdge[]
  axes?: { x?: DiagramAxis; y?: DiagramAxis }
  quadrants?: string[]
  center?: string
}

export const DW = 1000
export const DH = 560

function esc(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

export function wrapLabel(text: string, max = 18): string[] {
  const words = text.split(/\s+/)
  const lines: string[] = []
  let line = ''
  for (const word of words) {
    const candidate = line ? `${line} ${word}` : word
    if (candidate.length > max && line) {
      lines.push(line)
      line = word
    } else line = candidate
  }
  if (line) lines.push(line)
  return lines.slice(0, 3)
}

export function textLines(
  x: number,
  y: number,
  lines: string[],
  cls: string,
  anchor = 'middle',
  lineHeight = 22,
): string {
  return lines
    .map(
      (line, i) =>
        `<text class="${cls}" x="${x.toFixed(1)}" y="${(y + i * lineHeight).toFixed(1)}" text-anchor="${anchor}">${esc(line)}</text>`,
    )
    .join('')
}

export function networkDiagram(spec: DiagramSpec): string {
  const ring = spec.nodes.filter((node) => !node.hub)
  const hubs = spec.nodes.filter((node) => node.hub)
  const cx = 300
  const cy = DH / 2
  const r = 215
  const pos = new Map<string, [number, number]>()
  ring.forEach((node, i) => {
    const angle = -Math.PI / 2 + (i / ring.length) * Math.PI * 2
    pos.set(node.label, [cx + r * Math.cos(angle), cy + r * Math.sin(angle)])
  })
  const hubX = 740
  const hubGap = hubs.length > 1 ? Math.min(118, (DH - 100) / (hubs.length - 1)) : 0
  const hubY0 = cy - (hubGap * (hubs.length - 1)) / 2
  hubs.forEach((node, i) => pos.set(node.label, [hubX, hubY0 + i * hubGap]))
  const edges = (spec.edges ?? [])
    .map((edge, i) => {
      const a = pos.get(edge.from)
      const b = pos.get(edge.to)
      if (!a || !b) return ''
      const mx = (a[0] + b[0]) / 2 + 30
      return `<path class="edge" style="--i:${i}" d="M${a[0].toFixed(1)} ${a[1].toFixed(1)} Q ${mx.toFixed(1)} ${((a[1] + b[1]) / 2).toFixed(1)} ${b[0].toFixed(1)} ${b[1].toFixed(1)}"/>`
    })
    .join('')
  const ringSvg = `<circle class="guide" cx="${cx}" cy="${cy}" r="${r}"/>${ring
    .map((node, i) => {
      const [x, y] = pos.get(node.label) as [number, number]
      const outward = Math.atan2(y - cy, x - cx)
      const lx = x + Math.cos(outward) * 24
      const ly = y + Math.sin(outward) * 24
      const anchor = Math.cos(outward) > 0.3 ? 'start' : Math.cos(outward) < -0.3 ? 'end' : 'middle'
      return `<g class="node" style="--i:${i}"><circle class="dot" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="6"/>${textLines(lx, ly + 6, wrapLabel(node.label, 16), 'label', anchor, 18)}</g>`
    })
    .join('')}`
  const hubSvg = hubs
    .map((node, i) => {
      const [x, y] = pos.get(node.label) as [number, number]
      return `<g class="hub" style="--i:${i}"><rect x="${(x - 14).toFixed(1)}" y="${(y - 32).toFixed(1)}" width="260" height="64" rx="2"/><circle class="dot" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="7"/>${textLines(x + 20, y - 3, [node.label], 'hub-label', 'start')}${node.text ? textLines(x + 20, y + 20, [node.text], 'sub', 'start') : ''}</g>`
    })
    .join('')
  const centerSvg = spec.center ? textLines(cx, cy + 8, wrapLabel(spec.center, 14), 'center', 'middle', 28) : ''
  return `${edges}${ringSvg}${centerSvg}${hubSvg}`
}

export function matrixDiagram(spec: DiagramSpec): string {
  const left = 120
  const right = DW - 60
  const top = 40
  const bottom = DH - 90
  const w = right - left
  const h = bottom - top
  const midX = left + w / 2
  const midY = top + h / 2
  const q = spec.quadrants ?? []
  const quadrants = [
    [left + 16, top + 28, 'start'],
    [right - 16, top + 28, 'end'],
    [left + 16, bottom - 16, 'start'],
    [right - 16, bottom - 16, 'end'],
  ]
    .map(([x, y, anchor], i) =>
      q[i] ? `<text class="quadrant" x="${x}" y="${y}" text-anchor="${anchor}">${esc(q[i])}</text>` : '',
    )
    .join('')
  const xAxis = spec.axes?.x
  const yAxis = spec.axes?.y
  const axes = `<rect class="guide" x="${left}" y="${top}" width="${w}" height="${h}"/><line class="guide dashed" x1="${midX}" x2="${midX}" y1="${top}" y2="${bottom}"/><line class="guide dashed" x1="${left}" x2="${right}" y1="${midY}" y2="${midY}"/>${xAxis ? `<text class="axis" x="${midX}" y="${bottom + 60}" text-anchor="middle">${esc(xAxis.label)}</text>${xAxis.low ? `<text class="tick" x="${left}" y="${bottom + 30}" text-anchor="start">${esc(xAxis.low)}</text>` : ''}${xAxis.high ? `<text class="tick" x="${right}" y="${bottom + 30}" text-anchor="end">${esc(xAxis.high)}</text>` : ''}` : ''}${yAxis ? `<text class="axis" transform="translate(40 ${midY}) rotate(-90)" text-anchor="middle">${esc(yAxis.label)}</text>${yAxis.low ? `<text class="tick" transform="translate(76 ${bottom}) rotate(-90)" text-anchor="start">${esc(yAxis.low)}</text>` : ''}${yAxis.high ? `<text class="tick" transform="translate(76 ${top}) rotate(-90)" text-anchor="end">${esc(yAxis.high)}</text>` : ''}` : ''}`
  const points = spec.nodes
    .map((node, i) => {
      const px = left + ((node.x ?? 50) / 100) * w
      const py = bottom - ((node.y ?? 50) / 100) * h
      const r = 8 + ((node.value ?? 40) / 100) * 18
      const anchor = px > right - 220 ? 'end' : 'start'
      const lx = anchor === 'start' ? px + r + 14 : px - r - 14
      return `<g class="point" style="--i:${i}"><circle class="halo" cx="${px.toFixed(1)}" cy="${py.toFixed(1)}" r="${(r + 10).toFixed(1)}"/><circle class="dot" cx="${px.toFixed(1)}" cy="${py.toFixed(1)}" r="${r.toFixed(1)}"/>${textLines(lx, py + 7, [node.label], 'point-label', anchor)}${node.text ? textLines(lx, py + 30, [node.text], 'sub', anchor) : ''}</g>`
    })
    .join('')
  return `${axes}${quadrants}${points}`
}

export function radarDiagram(spec: DiagramSpec): string {
  const cx = DW / 2
  const cy = DH / 2 + 10
  const r = 215
  const n = Math.max(3, spec.nodes.length)
  const angle = (i: number) => -Math.PI / 2 + (i / n) * Math.PI * 2
  const at = (i: number, f: number) =>
    `${(cx + r * f * Math.cos(angle(i))).toFixed(1)},${(cy + r * f * Math.sin(angle(i))).toFixed(1)}`
  const rings = [0.25, 0.5, 0.75, 1]
    .map((f) => `<polygon class="guide" points="${spec.nodes.map((_, i) => at(i, f)).join(' ')}"/>`)
    .join('')
  const spokes = spec.nodes
    .map(
      (_, i) =>
        `<line class="guide" x1="${cx}" y1="${cy}" x2="${(cx + r * Math.cos(angle(i))).toFixed(1)}" y2="${(cy + r * Math.sin(angle(i))).toFixed(1)}"/>`,
    )
    .join('')
  const shape = spec.nodes.map((node, i) => at(i, (node.value ?? 50) / 100)).join(' ')
  const labels = spec.nodes
    .map((node, i) => {
      const lx = cx + (r + 34) * Math.cos(angle(i))
      const ly = cy + (r + 34) * Math.sin(angle(i))
      const anchor = Math.cos(angle(i)) > 0.3 ? 'start' : Math.cos(angle(i)) < -0.3 ? 'end' : 'middle'
      const [px, py] = at(i, (node.value ?? 50) / 100).split(',')
      return `<g class="node" style="--i:${i}">${textLines(lx, ly + 6, wrapLabel(node.label, 18), 'label', anchor, 18)}<circle class="dot" cx="${px}" cy="${py}" r="5"/></g>`
    })
    .join('')
  return `${rings}${spokes}<polygon class="shape" points="${shape}"/>${labels}`
}

export function loopDiagram(spec: DiagramSpec): string {
  const cx = DW / 2
  const cy = DH / 2
  const r = 200
  const n = Math.max(3, spec.nodes.length)
  const angle = (i: number) => -Math.PI / 2 + (i / n) * Math.PI * 2
  const gap = 0.16
  const arcs = spec.nodes
    .map((_, i) => {
      const a0 = angle(i) + gap
      const a1 = angle(i + 1) - gap
      const x0 = cx + r * Math.cos(a0)
      const y0 = cy + r * Math.sin(a0)
      const x1 = cx + r * Math.cos(a1)
      const y1 = cy + r * Math.sin(a1)
      const tangent = a1 + Math.PI / 2
      const hx = -10 * Math.cos(tangent)
      const hy = -10 * Math.sin(tangent)
      const nx = 6 * Math.cos(a1)
      const ny = 6 * Math.sin(a1)
      return `<path class="arc" style="--i:${i}" d="M${x0.toFixed(1)} ${y0.toFixed(1)} A ${r} ${r} 0 0 1 ${x1.toFixed(1)} ${y1.toFixed(1)}"/><path class="arrow" d="M${(x1 + hx + nx).toFixed(1)} ${(y1 + hy + ny).toFixed(1)} L${x1.toFixed(1)} ${y1.toFixed(1)} L${(x1 + hx - nx).toFixed(1)} ${(y1 + hy - ny).toFixed(1)}"/>`
    })
    .join('')
  const nodes = spec.nodes
    .map((node, i) => {
      const a = angle(i)
      const x = cx + r * Math.cos(a)
      const y = cy + r * Math.sin(a)
      const lx = cx + (r + 44) * Math.cos(a)
      const ly = cy + (r + 44) * Math.sin(a)
      const anchor = Math.cos(a) > 0.3 ? 'start' : Math.cos(a) < -0.3 ? 'end' : 'middle'
      return `<g class="node" style="--i:${i}"><circle class="dot" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="7"/><text class="index" x="${(cx + (r - 34) * Math.cos(a)).toFixed(1)}" y="${(cy + (r - 34) * Math.sin(a) + 5).toFixed(1)}" text-anchor="middle">${String(i + 1).padStart(2, '0')}</text>${textLines(lx, ly + 6, wrapLabel(node.label, 16), 'label-strong', anchor, 20)}</g>`
    })
    .join('')
  const center = spec.center ? textLines(cx, cy + 8, wrapLabel(spec.center, 14), 'center', 'middle', 30) : ''
  return `${arcs}${nodes}${center}`
}

export function ladderDiagram(spec: DiagramSpec): string {
  const n = Math.max(2, spec.nodes.length)
  const left = 70
  const right = DW - 70
  const top = 60
  const bottom = DH - 80
  const stepX = (right - left) / (n - 1)
  const stepY = (bottom - top) / (n - 1)
  const points = spec.nodes.map((_, i) => [left + i * stepX, bottom - i * stepY] as const)
  const path = points
    .map(([x, y], i) => (i === 0 ? `M${x.toFixed(1)} ${y.toFixed(1)}` : `H${x.toFixed(1)} V${y.toFixed(1)}`))
    .join(' ')
  const nodes = spec.nodes
    .map((node, i) => {
      const [x, y] = points[i]
      const above = i % 2 === 0
      const ly = above ? y - 22 : y + 34
      return `<g class="node" style="--i:${i}"><circle class="dot" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="6"/><text class="index" x="${x.toFixed(1)}" y="${(above ? y + 30 : y - 16).toFixed(1)}" text-anchor="middle">${String(i + 1).padStart(2, '0')}</text>${textLines(x, ly, wrapLabel(node.label, 14), 'label-strong', 'middle', 20)}</g>`
    })
    .join('')
  return `<path class="stair" d="${path}"/>${nodes}`
}

export function diagramSvg(spec: DiagramSpec, title?: string): string {
  const body =
    spec.kind === 'network'
      ? networkDiagram(spec)
      : spec.kind === 'matrix'
        ? matrixDiagram(spec)
        : spec.kind === 'radar'
          ? radarDiagram(spec)
          : spec.kind === 'loop'
            ? loopDiagram(spec)
            : ladderDiagram(spec)
  return `<svg class="diagram diagram-${spec.kind}" viewBox="0 0 ${DW} ${DH}" role="img"${title ? ` aria-label="${esc(title)}"` : ''}>${body}</svg>`
}
