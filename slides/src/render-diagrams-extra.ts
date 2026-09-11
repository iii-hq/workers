import { DH, type DiagramSpec, DW, esc, textLines, wrapLabel } from './render-diagrams.js'

export const WIDE_W = 1400

export function radialDiagram(spec: DiagramSpec): string {
  const hub = spec.nodes.find((node) => node.hub) ?? spec.nodes[0]
  const ring = spec.nodes.filter((node) => node !== hub)
  const cx = DW / 2
  const cy = 312
  const r = 190
  const groups: string[] = []
  for (const node of ring) if (node.group && !groups.includes(node.group)) groups.push(node.group)
  const angle = (i: number) => -Math.PI / 2 + ((i + 0.5) / ring.length) * Math.PI * 2
  const spokes = ring
    .map((node, i) => {
      const a = angle(i)
      const x = cx + r * Math.cos(a)
      const y = cy + r * Math.sin(a)
      const lx = cx + (r + 28) * Math.cos(a)
      const ly = cy + (r + 28) * Math.sin(a)
      const anchor = Math.cos(a) > 0.08 ? 'start' : Math.cos(a) < -0.08 ? 'end' : 'middle'
      return `<g class="node" style="--i:${i}"><line class="edge" x1="${(cx + 58 * Math.cos(a)).toFixed(1)}" y1="${(cy + 58 * Math.sin(a)).toFixed(1)}" x2="${x.toFixed(1)}" y2="${y.toFixed(1)}"/><circle class="dot" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="5"/>${textLines(lx, ly + 6, wrapLabel(node.label, 16), 'label', anchor, 18)}</g>`
    })
    .join('')
  const arcs = groups
    .map((group, gi) => {
      const indices = ring.map((node, i) => (node.group === group ? i : -1)).filter((i) => i >= 0)
      if (!indices.length) return ''
      const a0 = angle(indices[0]) - Math.PI / ring.length + 0.03
      const a1 = angle(indices[indices.length - 1]) + Math.PI / ring.length - 0.03
      const rr = r + 104
      const large = a1 - a0 > Math.PI ? 1 : 0
      const path = `M${(cx + rr * Math.cos(a0)).toFixed(1)} ${(cy + rr * Math.sin(a0)).toFixed(1)} A ${rr} ${rr} 0 ${large} 1 ${(cx + rr * Math.cos(a1)).toFixed(1)} ${(cy + rr * Math.sin(a1)).toFixed(1)}`
      const mid = (a0 + a1) / 2
      const lx = cx + (rr + 24) * Math.cos(mid)
      const ly = cy + (rr + 24) * Math.sin(mid)
      const anchor = Math.cos(mid) > 0.08 ? 'start' : Math.cos(mid) < -0.08 ? 'end' : 'middle'
      return `<g class="group" style="--i:${gi}"><path class="group-arc" d="${path}"/>${textLines(lx, ly + 5, [group], 'group-label', anchor)}</g>`
    })
    .join('')
  const center = `<g class="hub"><circle class="hub-disc" cx="${cx}" cy="${cy}" r="54"/>${textLines(cx, cy + 6, wrapLabel(hub.label, 12), 'center', 'middle', 26)}${hub.text ? textLines(cx, cy + 80, [hub.text], 'sub', 'middle') : ''}</g>`
  return `${arcs}${spokes}${center}`
}

export function coverageDiagram(spec: DiagramSpec): string {
  const rows = spec.nodes.filter((node) => node.hub)
  const columns = spec.nodes.filter((node) => !node.hub)
  const left = 320
  const right = WIDE_W - 70
  const top = 96
  const rowGap = 56
  const colGap = columns.length > 1 ? (right - left) / (columns.length - 1) : 0
  const weight = new Map<string, number>()
  for (const edge of spec.edges ?? []) weight.set(`${edge.from}|${edge.to}`, edge.weight ?? 1)
  const header = columns
    .map(
      (col, c) =>
        `<text class="col-label" x="${(left + c * colGap).toFixed(1)}" y="${top - 40}" text-anchor="middle">${esc(col.label)}</text>`,
    )
    .join('')
  const body = rows
    .map((row, r) => {
      const y = top + r * rowGap
      const cells = columns
        .map((col, c) => {
          const x = left + c * colGap
          const w = weight.get(`${row.label}|${col.label}`) ?? 0
          if (w >= 1) return `<circle class="dot" cx="${x.toFixed(1)}" cy="${y}" r="11"/>`
          if (w > 0)
            return `<circle class="hollow" cx="${x.toFixed(1)}" cy="${y}" r="11"/><path class="half" d="M${x.toFixed(1)} ${y - 11} A 11 11 0 0 1 ${x.toFixed(1)} ${y + 11} Z"/>`
          return `<line class="none" x1="${(x - 7).toFixed(1)}" x2="${(x + 7).toFixed(1)}" y1="${y}" y2="${y}"/>`
        })
        .join('')
      const band = row.emphasis
        ? `<rect class="row-emphasis" x="30" y="${y - rowGap / 2 + 4}" width="${WIDE_W - 60}" height="${rowGap - 8}" rx="3"/>`
        : ''
      return `<g class="hub${row.emphasis ? ' emphasis' : ''}" style="--i:${r}">${band}<line class="band" x1="30" x2="${WIDE_W - 30}" y1="${y + rowGap / 2 - 4}" y2="${y + rowGap / 2 - 4}"/>${textLines(left - 64, y + 9, [row.label], 'row-label', 'end')}${cells}</g>`
    })
    .join('')
  return `${header}${body}`
}

export function stackDiagram(spec: DiagramSpec): string {
  const columns = spec.nodes.filter((node) => !node.hub)
  const layers = spec.nodes.filter((node) => node.hub)
  const left = 70
  const right = DW - 70
  const layerH = 42
  const layersTop = DH - 30 - layers.length * (layerH + 6)
  const colTop = 60
  const colBottom = layersTop - 14
  const colGap = 14
  const colW = (right - left - colGap * (columns.length - 1)) / columns.length
  const cols = columns
    .map((node, i) => {
      const x = left + i * (colW + colGap)
      const label = wrapLabel(node.label, 10)
      return `<g class="node" style="--i:${i}"><rect class="pillar" x="${x.toFixed(1)}" y="${colTop}" width="${colW.toFixed(1)}" height="${colBottom - colTop}" rx="2"/><line class="pillar-cap" x1="${x.toFixed(1)}" x2="${(x + colW).toFixed(1)}" y1="${colTop}" y2="${colTop}"/>${textLines(x + colW / 2, colTop + 36, label, 'pillar-label', 'middle', 20)}</g>`
    })
    .join('')
  const bands = layers
    .map((node, i) => {
      const y = layersTop + i * (layerH + 6)
      return `<g class="hub" style="--i:${i}"><rect class="layer" x="${left - 20}" y="${y}" width="${right - left + 40}" height="${layerH}" rx="2" style="opacity:${(1 - i * 0.09).toFixed(2)}"/>${textLines(left, y + layerH / 2 + 6, [node.label], 'layer-label', 'start')}${node.text ? textLines(right, y + layerH / 2 + 6, [node.text], 'layer-text', 'end') : ''}</g>`
    })
    .join('')
  return `${cols}${bands}`
}

export function spansDiagram(spec: DiagramSpec): string {
  const scale = spec.nodes.filter((node) => !node.hub)
  const bars = spec.nodes.filter((node) => node.hub)
  const left = 330
  const right = WIDE_W - 250
  const top = 96
  const step = scale.length > 1 ? (right - left) / (scale.length - 1) : 0
  const barGap = Math.min(64, (DH - top - 60) / Math.max(1, bars.length))
  const ticks = scale
    .map((node, i) => {
      const x = left + i * step
      return `<g class="node" style="--i:${i}"><line class="guide" x1="${x.toFixed(1)}" x2="${x.toFixed(1)}" y1="${top - 10}" y2="${top + bars.length * barGap}"/><text class="col-label" transform="translate(${(x + 4).toFixed(1)} ${top - 26}) rotate(-38)" text-anchor="start">${esc(node.label)}</text></g>`
    })
    .join('')
  const rows = bars
    .map((node, i) => {
      const y = top + i * barGap + barGap / 2
      const from = Math.max(1, Math.min(scale.length, node.x ?? 1)) - 1
      const to = Math.max(1, Math.min(scale.length, node.y ?? scale.length)) - 1
      const x0 = left + Math.min(from, to) * step
      const x1 = left + Math.max(from, to) * step
      const cls = node.emphasis ? 'span emphasis' : 'span'
      return `<g class="hub" style="--i:${i}">${textLines(left - 40, y + 8, [node.label], node.emphasis ? 'row-label emphasis' : 'row-label', 'end')}<line class="${cls}" x1="${x0.toFixed(1)}" x2="${x1.toFixed(1)}" y1="${y.toFixed(1)}" y2="${y.toFixed(1)}"/><circle class="dot" cx="${x0.toFixed(1)}" cy="${y.toFixed(1)}" r="6"/><circle class="dot" cx="${x1.toFixed(1)}" cy="${y.toFixed(1)}" r="6"/>${node.text ? textLines(x1 + 22, y + 6, [node.text], 'sub', 'start') : ''}</g>`
    })
    .join('')
  return `${ticks}${rows}`
}

export function allocationDiagram(spec: DiagramSpec): string {
  const periods = spec.nodes.filter((node) => !node.hub)
  const series = spec.nodes.filter((node) => node.hub)
  const left = 90
  const right = DW - 300
  const top = 50
  const bottom = DH - 90
  const colGap = 36
  const colW = (right - left - colGap * (periods.length - 1)) / periods.length
  const weight = new Map<string, number>()
  for (const edge of spec.edges ?? []) weight.set(`${edge.from}|${edge.to}`, edge.weight ?? 0)
  const bars = periods
    .map((period, p) => {
      const x = left + p * (colW + colGap)
      const total = series.reduce((sum, s) => sum + (weight.get(`${s.label}|${period.label}`) ?? 0), 0) || 1
      let y = bottom
      const segments = series
        .map((s, si) => {
          const share = (weight.get(`${s.label}|${period.label}`) ?? 0) / total
          const h = share * (bottom - top)
          y -= h
          const label =
            share >= 0.12
              ? `<text class="alloc-value" x="${(x + colW / 2).toFixed(1)}" y="${(y + h / 2 + 7).toFixed(1)}" text-anchor="middle">${Math.round(share * 100)}%</text>`
              : ''
          return h > 0
            ? `<rect class="alloc alloc-${si}" x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${colW.toFixed(1)}" height="${h.toFixed(1)}"/>${label}`
            : ''
        })
        .join('')
      return `<g class="node" style="--i:${p}">${segments}${textLines(x + colW / 2, bottom + 34, [period.label], 'col-label', 'middle')}${period.text ? textLines(x + colW / 2, bottom + 56, wrapLabel(period.text, 18), 'sub', 'middle', 16) : ''}</g>`
    })
    .join('')
  const legend = series
    .map((s, si) => {
      const y = top + 10 + si * 40
      return `<g class="hub" style="--i:${si}"><rect class="alloc alloc-${si}" x="${right + 40}" y="${y}" width="22" height="22" rx="2"/>${textLines(right + 74, y + 17, [s.label], 'legend', 'start')}</g>`
    })
    .join('')
  return `<line class="guide" x1="${left}" x2="${right}" y1="${bottom}" y2="${bottom}"/>${bars}${legend}`
}

export function flowDiagram(spec: DiagramSpec): string {
  const sources = spec.nodes.filter((node) => node.hub)
  const targets = spec.nodes.filter((node) => !node.hub)
  const left = 380
  const right = WIDE_W - 380
  const top = 50
  const bottom = DH - 40
  const total = (spec.edges ?? []).reduce((sum, e) => sum + (e.weight ?? 1), 0) || 1
  const outS = new Map<string, number>()
  const inT = new Map<string, number>()
  for (const e of spec.edges ?? []) {
    outS.set(e.from, (outS.get(e.from) ?? 0) + (e.weight ?? 1))
    inT.set(e.to, (inT.get(e.to) ?? 0) + (e.weight ?? 1))
  }
  const height = bottom - top
  const gapS = 14
  const gapT = 14
  const scaleS = (height - gapS * (sources.length - 1)) / total
  const scaleT = (height - gapT * (targets.length - 1)) / total
  const sPos = new Map<string, { y: number; h: number; cursor: number }>()
  const tPos = new Map<string, { y: number; h: number; cursor: number }>()
  let y = top
  for (const s of sources) {
    const h = (outS.get(s.label) ?? 0) * scaleS
    sPos.set(s.label, { y, h, cursor: y })
    y += h + gapS
  }
  y = top
  for (const t of targets) {
    const h = (inT.get(t.label) ?? 0) * scaleT
    tPos.set(t.label, { y, h, cursor: y })
    y += h + gapT
  }
  const ribbons = (spec.edges ?? [])
    .map((e, i) => {
      const s = sPos.get(e.from)
      const t = tPos.get(e.to)
      if (!s || !t) return ''
      const hs = (e.weight ?? 1) * scaleS
      const ht = (e.weight ?? 1) * scaleT
      const y0 = s.cursor
      const y1 = t.cursor
      s.cursor += hs
      t.cursor += ht
      const x0 = left + 18
      const x1 = right - 18
      const mx = (x0 + x1) / 2
      return `<path class="ribbon" style="--i:${i}" d="M${x0} ${y0.toFixed(1)} C ${mx} ${y0.toFixed(1)}, ${mx} ${y1.toFixed(1)}, ${x1} ${y1.toFixed(1)} L${x1} ${(y1 + ht).toFixed(1)} C ${mx} ${(y1 + ht).toFixed(1)}, ${mx} ${(y0 + hs).toFixed(1)}, ${x0} ${(y0 + hs).toFixed(1)} Z"/>`
    })
    .join('')
  const sBars = sources
    .map((s, i) => {
      const p = sPos.get(s.label) as { y: number; h: number }
      return `<g class="hub" style="--i:${i}"><rect class="flow-bar" x="${left}" y="${p.y.toFixed(1)}" width="18" height="${Math.max(2, p.h).toFixed(1)}"/>${textLines(left - 16, p.y + p.h / 2 + 8, wrapLabel(s.label, 18), 'row-label', 'end', 24)}</g>`
    })
    .join('')
  const tBars = targets
    .map((t, i) => {
      const p = tPos.get(t.label) as { y: number; h: number }
      return `<g class="node" style="--i:${i}"><rect class="flow-bar" x="${right - 18}" y="${p.y.toFixed(1)}" width="18" height="${Math.max(2, p.h).toFixed(1)}"/>${textLines(right + 16, p.y + p.h / 2 + 8, wrapLabel(t.label, 18), 'row-label', 'start', 24)}</g>`
    })
    .join('')
  return `${ribbons}${sBars}${tBars}`
}
