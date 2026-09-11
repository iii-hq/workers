export type ChartKind = 'bar' | 'line' | 'donut'
export type Visual = 'none' | 'orbits' | 'grid' | 'waves' | 'arcs' | 'rings'
export interface Series {
  label: string
  value: number
}

function esc(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

export function formatValue(value: number, unit?: string): string {
  const abs = Math.abs(value)
  const text =
    abs >= 1_000_000_000
      ? `${(value / 1_000_000_000).toFixed(abs >= 10_000_000_000 ? 0 : 1)}B`
      : abs >= 1_000_000
        ? `${(value / 1_000_000).toFixed(abs >= 10_000_000 ? 0 : 1)}M`
        : abs >= 10_000
          ? `${(value / 1_000).toFixed(0)}K`
          : Number.isInteger(value)
            ? String(value)
            : value.toFixed(1)
  if (!unit) return text
  return unit === '%' || unit === 'x' ? `${text}${unit}` : /^[$€£¥]$/.test(unit) ? `${unit}${text}` : `${text} ${unit}`
}

const W = 1000
const H = 480

function barChart(series: Series[], unit?: string): string {
  const max = Math.max(...series.map((s) => Math.abs(s.value)), 1)
  const padL = 24
  const padB = 70
  const top = 60
  const plotH = H - padB - top
  const gap = 18
  const barW = Math.min(140, (W - padL * 2 - gap * (series.length - 1)) / series.length)
  const totalW = barW * series.length + gap * (series.length - 1)
  const startX = (W - totalW) / 2
  const bars = series
    .map((point, index) => {
      const h = Math.max(6, (Math.abs(point.value) / max) * plotH)
      const x = startX + index * (barW + gap)
      const y = top + plotH - h
      return `<g class="bar" style="--i:${index}"><rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${barW.toFixed(1)}" height="${h.toFixed(1)}" rx="14" fill="url(#barfill)"/><text class="chart-value" x="${(x + barW / 2).toFixed(1)}" y="${(y - 16).toFixed(1)}" text-anchor="middle">${esc(formatValue(point.value, unit))}</text><text class="chart-label" x="${(x + barW / 2).toFixed(1)}" y="${H - 28}" text-anchor="middle">${esc(point.label)}</text></g>`
    })
    .join('')
  return `<line class="chart-axis" x1="${padL}" x2="${W - padL}" y1="${top + plotH}" y2="${top + plotH}"/>${bars}`
}

function lineChart(series: Series[], unit?: string): string {
  const values = series.map((s) => s.value)
  const max = Math.max(...values)
  const min = Math.min(0, ...values)
  const range = max - min || 1
  const padL = 40
  const padR = 40
  const top = 60
  const padB = 70
  const plotW = W - padL - padR
  const plotH = H - padB - top
  const step = series.length > 1 ? plotW / (series.length - 1) : 0
  const points = series.map(
    (point, index) => [padL + index * step, top + plotH - ((point.value - min) / range) * plotH] as const,
  )
  const path = points.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`).join(' ')
  const area = `${path} L${points[points.length - 1][0].toFixed(1)},${top + plotH} L${padL},${top + plotH} Z`
  const dots = points
    .map(
      ([x, y], i) =>
        `<g class="dot" style="--i:${i}"><circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="9"/><text class="chart-value" x="${x.toFixed(1)}" y="${(y - 22).toFixed(1)}" text-anchor="middle">${esc(formatValue(series[i].value, unit))}</text><text class="chart-label" x="${x.toFixed(1)}" y="${H - 28}" text-anchor="middle">${esc(series[i].label)}</text></g>`,
    )
    .join('')
  return `<line class="chart-axis" x1="${padL}" x2="${W - padR}" y1="${top + plotH}" y2="${top + plotH}"/><path class="chart-area" d="${area}" fill="url(#areafill)"/><path class="chart-line" d="${path}"/>${dots}`
}

function donutChart(series: Series[], unit?: string): string {
  const total = series.reduce((sum, s) => sum + Math.max(0, s.value), 0) || 1
  const cx = 250
  const cy = H / 2
  const r = 170
  const stroke = 46
  const circumference = 2 * Math.PI * r
  let offset = 0
  const arcs = series
    .map((point, index) => {
      const fraction = Math.max(0, point.value) / total
      const length = fraction * circumference
      const dash = `${length.toFixed(2)} ${(circumference - length).toFixed(2)}`
      const el = `<circle class="arc" style="--i:${index}" cx="${cx}" cy="${cy}" r="${r}" fill="none" stroke-width="${stroke}" stroke-dasharray="${dash}" stroke-dashoffset="${(-offset).toFixed(2)}" stroke="var(--chart-${index % 6})" transform="rotate(-90 ${cx} ${cy})"/>`
      offset += length
      return el
    })
    .join('')
  const legend = series
    .map((point, index) => {
      const y = cy - ((series.length - 1) * 56) / 2 + index * 56
      return `<g class="legend" style="--i:${index}"><rect x="520" y="${(y - 14).toFixed(1)}" width="28" height="28" rx="8" fill="var(--chart-${index % 6})"/><text class="chart-legend" x="568" y="${(y + 8).toFixed(1)}">${esc(point.label)}</text><text class="chart-value" x="${W - 40}" y="${(y + 8).toFixed(1)}" text-anchor="end">${esc(formatValue(point.value, unit))}${unit ? '' : ` <tspan class="chart-pct">${Math.round((Math.max(0, point.value) / total) * 100)}%</tspan>`}</text></g>`
    })
    .join('')
  return `${arcs}<text class="chart-total" x="${cx}" y="${cy + 14}" text-anchor="middle">${esc(formatValue(total, unit))}</text>${legend}`
}

export function chartSvg(kind: ChartKind, series: Series[], unit?: string, title?: string): string {
  const body =
    kind === 'line' ? lineChart(series, unit) : kind === 'donut' ? donutChart(series, unit) : barChart(series, unit)
  return `<svg class="chart chart-${kind}" viewBox="0 0 ${W} ${H}" role="img"${title ? ` aria-label="${esc(title)}"` : ''}><defs><linearGradient id="barfill" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="var(--accent)"/><stop offset="1" stop-color="var(--accent-2)"/></linearGradient><linearGradient id="areafill" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="var(--accent)" stop-opacity=".35"/><stop offset="1" stop-color="var(--accent)" stop-opacity="0"/></linearGradient></defs>${body}</svg>`
}

export function motifSvg(visual: Visual | undefined): string {
  if (!visual || visual === 'none') return ''
  let body = ''
  switch (visual) {
    case 'orbits':
      body = `<g class="spin"><ellipse cx="500" cy="500" rx="420" ry="160"/><ellipse cx="500" cy="500" rx="420" ry="160" transform="rotate(60 500 500)"/><ellipse cx="500" cy="500" rx="420" ry="160" transform="rotate(120 500 500)"/></g><circle class="fill" cx="500" cy="500" r="26"/><circle class="fill" cx="920" cy="500" r="12"/><circle class="fill" cx="290" cy="136" r="10"/>`
      break
    case 'grid':
      body = Array.from({ length: 10 }, (_, row) =>
        Array.from(
          { length: 10 },
          (_, col) =>
            `<circle class="fill" cx="${100 + col * 88}" cy="${100 + row * 88}" r="${3 + ((row + col) % 3)}" opacity="${(0.25 + ((row * 7 + col * 3) % 10) / 14).toFixed(2)}"/>`,
        ).join(''),
      ).join('')
      break
    case 'waves':
      body = Array.from(
        { length: 7 },
        (_, i) =>
          `<path d="M0 ${360 + i * 60} C 200 ${300 + i * 60}, 300 ${440 + i * 60}, 500 ${380 + i * 60} S 800 ${300 + i * 60}, 1000 ${400 + i * 60}" opacity="${(1 - i * 0.12).toFixed(2)}"/>`,
      ).join('')
      break
    case 'arcs':
      body = Array.from(
        { length: 6 },
        (_, i) =>
          `<path d="M ${1000} ${1000 - (i + 1) * 150} A ${(i + 1) * 150} ${(i + 1) * 150} 0 0 0 ${1000 - (i + 1) * 150} 1000" opacity="${(1 - i * 0.13).toFixed(2)}"/>`,
      ).join('')
      break
    case 'rings':
      body =
        Array.from(
          { length: 6 },
          (_, i) => `<circle cx="500" cy="500" r="${90 + i * 75}" opacity="${(1 - i * 0.14).toFixed(2)}"/>`,
        ).join('') + `<circle class="fill" cx="500" cy="500" r="18"/>`
      break
  }
  return `<svg class="motif motif-${visual}" viewBox="0 0 1000 1000" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2">${body}</svg>`
}
