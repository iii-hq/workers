import type { CSSProperties, ReactNode } from 'react'
import { ICON_PATHS } from '../../src/deck-icons'
import { formatValue } from '../../src/render-visuals'
import type { ChartKind, Series, Visual } from './types'

export function DeckIcon({ name, className }: { name: string; className?: string }) {
  const path = ICON_PATHS[name]
  if (!path) return null
  return (
    <svg
      className={className ?? 'sl-icon'}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={path} />
    </svg>
  )
}

const W = 1000
const H = 480

function Bars({ series, unit }: { series: Series[]; unit?: string }) {
  const max = Math.max(...series.map((s) => Math.abs(s.value)), 1)
  const padL = 24
  const padB = 70
  const top = 60
  const plotH = H - padB - top
  const gap = 18
  const barW = Math.min(140, (W - padL * 2 - gap * (series.length - 1)) / series.length)
  const startX = (W - (barW * series.length + gap * (series.length - 1))) / 2
  return (
    <>
      <line className="sl-chart-axis" x1={padL} x2={W - padL} y1={top + plotH} y2={top + plotH} />
      {series.map((point, index) => {
        const h = Math.max(6, (Math.abs(point.value) / max) * plotH)
        const x = startX + index * (barW + gap)
        const y = top + plotH - h
        return (
          <g key={`${point.label}-${index}`} className="sl-chart-bar" style={{ '--i': index } as CSSProperties}>
            <rect x={x} y={y} width={barW} height={h} rx={14} fill="url(#sl-barfill)" />
            <text className="sl-chart-value" x={x + barW / 2} y={y - 16} textAnchor="middle">
              {formatValue(point.value, unit)}
            </text>
            <text className="sl-chart-label" x={x + barW / 2} y={H - 28} textAnchor="middle">
              {point.label}
            </text>
          </g>
        )
      })}
    </>
  )
}

function Line({ series, unit }: { series: Series[]; unit?: string }) {
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
  return (
    <>
      <line className="sl-chart-axis" x1={padL} x2={W - padR} y1={top + plotH} y2={top + plotH} />
      <path d={area} fill="url(#sl-areafill)" />
      <path className="sl-chart-line" d={path} />
      {points.map(([x, y], i) => (
        <g key={`${series[i].label}-${i}`} className="sl-chart-dot">
          <circle cx={x} cy={y} r={9} />
          <text className="sl-chart-value" x={x} y={y - 22} textAnchor="middle">
            {formatValue(series[i].value, unit)}
          </text>
          <text className="sl-chart-label" x={x} y={H - 28} textAnchor="middle">
            {series[i].label}
          </text>
        </g>
      ))}
    </>
  )
}

function Donut({ series, unit }: { series: Series[]; unit?: string }) {
  const total = series.reduce((sum, s) => sum + Math.max(0, s.value), 0) || 1
  const cx = 250
  const cy = H / 2
  const r = 170
  const circumference = 2 * Math.PI * r
  let offset = 0
  return (
    <>
      {series.map((point, index) => {
        const length = (Math.max(0, point.value) / total) * circumference
        const el = (
          <circle
            key={`${point.label}-${index}`}
            cx={cx}
            cy={cy}
            r={r}
            fill="none"
            strokeWidth={46}
            strokeDasharray={`${length.toFixed(2)} ${(circumference - length).toFixed(2)}`}
            strokeDashoffset={(-offset).toFixed(2)}
            stroke={`var(--sl-chart-${index % 6})`}
            transform={`rotate(-90 ${cx} ${cy})`}
          />
        )
        offset += length
        return el
      })}
      <text className="sl-chart-total" x={cx} y={cy + 14} textAnchor="middle">
        {formatValue(total, unit)}
      </text>
      {series.map((point, index) => {
        const y = cy - ((series.length - 1) * 56) / 2 + index * 56
        return (
          <g key={`${point.label}-legend-${index}`}>
            <rect x={520} y={y - 14} width={28} height={28} rx={8} fill={`var(--sl-chart-${index % 6})`} />
            <text className="sl-chart-legend" x={568} y={y + 8}>
              {point.label}
            </text>
            <text className="sl-chart-value" x={W - 40} y={y + 8} textAnchor="end">
              {formatValue(point.value, unit)}
            </text>
          </g>
        )
      })}
    </>
  )
}

export function Chart({
  kind,
  series,
  unit,
  title,
}: {
  kind: ChartKind
  series: Series[]
  unit?: string
  title?: string
}) {
  if (!series.length) return <div className="sl-image-missing">Add series in the inspector</div>
  return (
    <svg
      className={`sl-chart sl-chart-${kind}`}
      viewBox={`0 0 ${W} ${H}`}
      role="img"
      aria-label={title ?? `${kind} chart`}
    >
      <defs>
        <linearGradient id="sl-barfill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="var(--sl-accent)" />
          <stop offset="1" stopColor="var(--sl-accent-2)" />
        </linearGradient>
        <linearGradient id="sl-areafill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="var(--sl-accent)" stopOpacity={0.35} />
          <stop offset="1" stopColor="var(--sl-accent)" stopOpacity={0} />
        </linearGradient>
      </defs>
      {kind === 'line' ? (
        <Line series={series} unit={unit} />
      ) : kind === 'donut' ? (
        <Donut series={series} unit={unit} />
      ) : (
        <Bars series={series} unit={unit} />
      )}
    </svg>
  )
}

export function Motif({ visual }: { visual?: Visual }) {
  if (!visual || visual === 'none') return null
  let body: ReactNode = null
  if (visual === 'orbits') {
    body = (
      <>
        <g className="sl-spin">
          <ellipse cx={500} cy={500} rx={420} ry={160} />
          <ellipse cx={500} cy={500} rx={420} ry={160} transform="rotate(60 500 500)" />
          <ellipse cx={500} cy={500} rx={420} ry={160} transform="rotate(120 500 500)" />
        </g>
        <circle className="sl-fill" cx={500} cy={500} r={26} />
        <circle className="sl-fill" cx={920} cy={500} r={12} />
        <circle className="sl-fill" cx={290} cy={136} r={10} />
      </>
    )
  } else if (visual === 'grid') {
    body = Array.from({ length: 100 }, (_, i) => {
      const row = Math.floor(i / 10)
      const col = i % 10
      return (
        <circle
          key={i}
          className="sl-fill"
          cx={100 + col * 88}
          cy={100 + row * 88}
          r={3 + ((row + col) % 3)}
          opacity={0.25 + ((row * 7 + col * 3) % 10) / 14}
        />
      )
    })
  } else if (visual === 'waves') {
    body = Array.from({ length: 7 }, (_, i) => (
      <path
        key={i}
        d={`M0 ${360 + i * 60} C 200 ${300 + i * 60}, 300 ${440 + i * 60}, 500 ${380 + i * 60} S 800 ${300 + i * 60}, 1000 ${400 + i * 60}`}
        opacity={1 - i * 0.12}
      />
    ))
  } else if (visual === 'arcs') {
    body = Array.from({ length: 6 }, (_, i) => (
      <path
        key={i}
        d={`M 1000 ${1000 - (i + 1) * 150} A ${(i + 1) * 150} ${(i + 1) * 150} 0 0 0 ${1000 - (i + 1) * 150} 1000`}
        opacity={1 - i * 0.13}
      />
    ))
  } else if (visual === 'rings') {
    body = (
      <>
        {Array.from({ length: 6 }, (_, i) => (
          <circle key={i} cx={500} cy={500} r={90 + i * 75} opacity={1 - i * 0.14} />
        ))}
        <circle className="sl-fill" cx={500} cy={500} r={18} />
      </>
    )
  }
  return (
    <svg
      className={`sl-motif sl-motif-${visual}`}
      viewBox="0 0 1000 1000"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
    >
      {body}
    </svg>
  )
}
