export type DiagramKind = 'network' | 'matrix' | 'radar' | 'loop' | 'ladder' | 'weave' | 'gate'

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
  const cx = hubs.length ? 300 : DW / 2
  const cy = DH / 2
  const r = hubs.length ? 215 : 232
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
      const ringToRing = !hubs.some((h) => h.label === edge.from || h.label === edge.to)
      const mx = ringToRing ? cx + ((a[0] + b[0]) / 2 - cx) * 0.25 : (a[0] + b[0]) / 2 + 30
      const my = ringToRing ? cy + ((a[1] + b[1]) / 2 - cy) * 0.25 : (a[1] + b[1]) / 2
      return `<path class="edge" style="--i:${i}" d="M${a[0].toFixed(1)} ${a[1].toFixed(1)} Q ${mx.toFixed(1)} ${my.toFixed(1)} ${b[0].toFixed(1)} ${b[1].toFixed(1)}"/>`
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

const MATRIX_W = 1400

export function matrixDiagram(spec: DiagramSpec): string {
  const left = 130
  const right = MATRIX_W - 60
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
    .map(([x, y, anchor], i) => (q[i] ? `<text class="quadrant" x="${x}" y="${y}" text-anchor="${anchor}">${esc(q[i])}</text>` : ''))
    .join('')
  const xAxis = spec.axes?.x
  const yAxis = spec.axes?.y
  const axes = `<rect class="guide" x="${left}" y="${top}" width="${w}" height="${h}"/><line class="guide dashed" x1="${midX}" x2="${midX}" y1="${top}" y2="${bottom}"/><line class="guide dashed" x1="${left}" x2="${right}" y1="${midY}" y2="${midY}"/>${xAxis ? `<text class="axis" x="${midX}" y="${bottom + 60}" text-anchor="middle">${esc(xAxis.label)}</text>${xAxis.low ? `<text class="tick" x="${left}" y="${bottom + 30}" text-anchor="start">${esc(xAxis.low)}</text>` : ''}${xAxis.high ? `<text class="tick" x="${right}" y="${bottom + 30}" text-anchor="end">${esc(xAxis.high)}</text>` : ''}` : ''}${yAxis ? `<text class="axis" transform="translate(44 ${midY}) rotate(-90)" text-anchor="middle">${esc(yAxis.label)}</text>${yAxis.low ? `<text class="tick" transform="translate(84 ${bottom}) rotate(-90)" text-anchor="start">${esc(yAxis.low)}</text>` : ''}${yAxis.high ? `<text class="tick" transform="translate(84 ${top}) rotate(-90)" text-anchor="end">${esc(yAxis.high)}</text>` : ''}` : ''}`
  const place = (node: DiagramNode) => [left + ((node.x ?? 50) / 100) * w, bottom - ((node.y ?? 50) / 100) * h] as const
  const byLabel = new Map(spec.nodes.map((node) => [node.label, place(node)]))
  const sequenced = (spec.edges ?? []).length > 0
  const trail = (spec.edges ?? [])
    .map((edge, i) => {
      const a = byLabel.get(edge.from)
      const b = byLabel.get(edge.to)
      if (!a || !b) return ''
      return `<line class="trail" style="--i:${i}" x1="${a[0].toFixed(1)}" y1="${a[1].toFixed(1)}" x2="${b[0].toFixed(1)}" y2="${b[1].toFixed(1)}"/>`
    })
    .join('')
  const points = spec.nodes
    .map((node, i) => {
      const [px, py] = place(node)
      const r = sequenced ? 7 : 8 + ((node.value ?? 40) / 100) * 18
      const anchor = sequenced ? (i % 2 === 0 ? 'start' : 'end') : px > right - 240 ? 'end' : 'start'
      const lx = anchor === 'start' ? px + r + 14 : px - r - 14
      const label = sequenced ? `${String(i + 1).padStart(2, '0')}  ${node.label}` : node.label
      const halo = sequenced ? '' : `<circle class="halo" cx="${px.toFixed(1)}" cy="${py.toFixed(1)}" r="${(r + 10).toFixed(1)}"/>`
      const sub = node.text && !sequenced ? textLines(lx, py + 30, [node.text], 'sub', anchor) : ''
      return `<g class="point" style="--i:${i}">${halo}<circle class="dot" cx="${px.toFixed(1)}" cy="${py.toFixed(1)}" r="${r.toFixed(1)}"/>${textLines(lx, py + 7, [label], sequenced ? 'point-label small' : 'point-label', anchor)}${sub}</g>`
    })
    .join('')
  return `${axes}${quadrants}${trail}${points}`
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
  const top = 70
  const bottom = DH - 90
  const stepX = (right - left) / (n - 1)
  const stepY = (bottom - top) / (n - 1)
  const points = spec.nodes.map((_, i) => [left + i * stepX, bottom - i * stepY] as const)
  const path = points
    .map(([x, y], i) => (i === 0 ? `M${x.toFixed(1)} ${y.toFixed(1)}` : `H${x.toFixed(1)} V${y.toFixed(1)}`))
    .join(' ')
  const fill = `${path} H${right + 20} V${bottom + 40} H${left} Z`
  const nodes = spec.nodes
    .map((node, i) => {
      const [x, y] = points[i]
      const above = i % 2 === 0
      const label = wrapLabel(node.label, 12)
      const ly = above ? y - 26 - (label.length - 1) * 22 : y + 40
      const hi = node.hub ? ' highlight' : ''
      return `<g class="node${hi}" style="--i:${i}"><circle class="dot" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="${node.hub ? 10 : 6}"/><text class="index" x="${x.toFixed(1)}" y="${(above ? y + 30 : y - 18).toFixed(1)}" text-anchor="middle">${String(i + 1).padStart(2, '0')}</text>${textLines(x, ly, label, 'label-strong', 'middle', 22)}${node.text ? textLines(x, above ? ly + label.length * 22 : ly + label.length * 22, [node.text], 'sub', 'middle') : ''}</g>`
    })
    .join('')
  return `<path class="stair-fill" d="${fill}"/><path class="stair" d="${path}"/>${nodes}`
}

const WEAVE_H = 520
const WEAVE_W = 1400

export function weaveDiagram(spec: DiagramSpec): string {
  const columns = spec.nodes.filter((node) => !node.hub)
  const rows = spec.nodes.filter((node) => node.hub)
  const left = 230
  const right = WEAVE_W - 190
  const top = 150
  const bottom = WEAVE_H - 40
  const colGap = columns.length > 1 ? (right - left) / (columns.length - 1) : 0
  const rowGap = rows.length > 1 ? (bottom - top) / (rows.length - 1) : 0
  const hasEdges = (spec.edges ?? []).length > 0
  const marked = new Set((spec.edges ?? []).flatMap((edge) => [`${edge.from}|${edge.to}`, `${edge.to}|${edge.from}`]))
  const isOn = (row: string, col: string) => !hasEdges || marked.has(`${row}|${col}`)
  const dense = columns.length > 7
  const colSvg = columns
    .map((node, i) => {
      const x = left + i * colGap
      const label = wrapLabel(node.label, 12)
      const heading = dense
        ? `<text class="col-label" transform="translate(${(x + 4).toFixed(1)} ${top - 26}) rotate(-38)" text-anchor="start">${esc(node.label)}</text>`
        : textLines(x, top - 30 - (label.length - 1) * 16, label, 'col-label', 'middle', 16)
      return `<g class="node" style="--i:${i}"><line class="guide" x1="${x.toFixed(1)}" x2="${x.toFixed(1)}" y1="${top - 14}" y2="${bottom + 10}"/>${heading}${node.text ? textLines(x, bottom + 34, [node.text], 'sub', 'middle') : ''}</g>`
    })
    .join('')
  const rowSvg = rows
    .map((node, r) => {
      const y = top + r * rowGap
      const dots = columns
        .map((col, c) => {
          const x = left + c * colGap
          if (!isOn(node.label, col.label))
            return `<circle class="hollow" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="3.5"/>`
          const inherited = hasEdges && columns.slice(0, c).some((prev) => isOn(node.label, prev.label))
          return inherited
            ? `<circle class="inherited" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="9"/>`
            : `<circle class="dot" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="9"/>`
        })
        .join('')
      return `<g class="hub" style="--i:${r}"><line class="band" x1="${left - 24}" x2="${right}" y1="${y.toFixed(1)}" y2="${y.toFixed(1)}"/>${textLines(left - 44, y + 8, [node.label], 'row-label', 'end')}${node.text ? textLines(left - 44, y + 27, [node.text], 'sub', 'end') : ''}${dots}</g>`
    })
    .join('')
  return `${colSvg}${rowSvg}`
}
export function gateDiagram(spec: DiagramSpec): string {
  const stages = spec.nodes.filter((node) => !node.hub)
  const outcomes = spec.nodes.filter((node) => node.hub)
  const y = 186
  const left = 60
  const gateX = DW - 330
  const stageGap = stages.length > 1 ? (gateX - 120 - left) / (stages.length - 1) : 0
  const line = `<line class="rail" x1="${left}" x2="${gateX - 44}" y1="${y}" y2="${y}"/>`
  const stageSvg = stages
    .map((node, i) => {
      const x = left + i * stageGap
      const label = wrapLabel(node.label, 14)
      return `<g class="node" style="--i:${i}"><circle class="dot" cx="${x.toFixed(1)}" cy="${y}" r="8"/><text class="index" x="${x.toFixed(1)}" y="${y - 34}" text-anchor="middle">${String(i + 1).padStart(2, '0')}</text>${textLines(x, y + 46, label, 'label-strong', 'middle', 24)}${node.text ? textLines(x, y + 48 + label.length * 24, wrapLabel(node.text, 22), 'sub-plain', 'middle', 18) : ''}</g>`
    })
    .join('')
  const d = 40
  const gate = `<g class="gate"><polygon class="diamond" points="${gateX},${y - d} ${gateX + d},${y} ${gateX},${y + d} ${gateX - d},${y}"/>${spec.center ? textLines(gateX, y - d - 18, [spec.center], 'index', 'middle') : ''}</g>`
  const outGap = outcomes.length > 1 ? Math.min(130, 300 / (outcomes.length - 1)) : 0
  const outY0 = y - (outGap * (outcomes.length - 1)) / 2
  const outX = gateX + 120
  const outSvg = outcomes
    .map((node, i) => {
      const oy = outY0 + i * outGap
      const cx = gateX + d
      const mx = cx + 40
      return `<g class="hub" style="--i:${i}"><path class="branch" d="M${cx} ${y} C ${mx} ${y}, ${mx} ${oy.toFixed(1)}, ${outX - 10} ${oy.toFixed(1)}"/><circle class="dot" cx="${outX}" cy="${oy.toFixed(1)}" r="7"/>${textLines(outX + 18, oy + 8, [node.label], 'label-strong', 'start')}${node.text ? textLines(outX + 18, oy + 32, wrapLabel(node.text, 30), 'sub-plain', 'start', 18) : ''}</g>`
    })
    .join('')
  return `${line}${stageSvg}${gate}${outSvg}`
}

export function diagramSvg(spec: DiagramSpec, title?: string): string {
  const renderers: Record<DiagramKind, (s: DiagramSpec) => string> = {
    network: networkDiagram,
    matrix: matrixDiagram,
    radar: radarDiagram,
    loop: loopDiagram,
    ladder: ladderDiagram,
    weave: weaveDiagram,
    gate: gateDiagram,
  }
  const body = (renderers[spec.kind] ?? networkDiagram)(spec)
  const height = spec.kind === 'weave' ? WEAVE_H : spec.kind === 'gate' ? 420 : DH
  const width = spec.kind === 'weave' ? WEAVE_W : spec.kind === 'matrix' ? MATRIX_W : DW
  return `<svg class="diagram diagram-${spec.kind}" viewBox="0 0 ${width} ${height}" role="img"${title ? ` aria-label="${esc(title)}"` : ''}>${body}</svg>`
}
