import { type CSSProperties, type KeyboardEvent, type ReactElement, useEffect, useRef, useState } from 'react'
import type { Block, Entry, Slide, Theme, ThemeOverrides } from './types'
import { Chart, DeckIcon, Motif } from './visuals'

export const CANVAS_W = 1600
export const CANVAS_H = 900

function hexToRgb(hex: string): [number, number, number] {
  let value = hex.trim().replace('#', '')
  if (value.length === 3)
    value = value
      .split('')
      .map((c) => c + c)
      .join('')
  const int = Number.parseInt(value, 16)
  return [(int >> 16) & 255, (int >> 8) & 255, int & 255]
}

function luminance(hex: string): number {
  const [r, g, b] = hexToRgb(hex).map((channel) => {
    const c = channel / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function mix(a: string, b: string, amount: number): string {
  const [ar, ag, ab] = hexToRgb(a)
  const [br, bg, bb] = hexToRgb(b)
  const channel = (x: number, y: number) => Math.round(x + (y - x) * amount)
  return `#${[channel(ar, br), channel(ag, bg), channel(ab, bb)].map((c) => c.toString(16).padStart(2, '0')).join('')}`
}

function alpha(hex: string, value: number): string {
  const [r, g, b] = hexToRgb(hex)
  return `rgba(${r}, ${g}, ${b}, ${value})`
}

function hueShift(hex: string, degrees: number): string {
  const [r, g, b] = hexToRgb(hex).map((c) => c / 255)
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const l = (max + min) / 2
  const d = max - min
  let h = 0
  const sat = d === 0 ? 0 : d / (1 - Math.abs(2 * l - 1))
  if (d !== 0) {
    if (max === r) h = ((g - b) / d) % 6
    else if (max === g) h = (b - r) / d + 2
    else h = (r - g) / d + 4
  }
  h = (((h * 60 + degrees) % 360) + 360) % 360
  const c = (1 - Math.abs(2 * l - 1)) * sat
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1))
  const m = l - c / 2
  const [r1, g1, b1] =
    h < 60
      ? [c, x, 0]
      : h < 120
        ? [x, c, 0]
        : h < 180
          ? [0, c, x]
          : h < 240
            ? [0, x, c]
            : h < 300
              ? [x, 0, c]
              : [c, 0, x]
  return `#${[r1, g1, b1]
    .map((v) =>
      Math.round((v + m) * 255)
        .toString(16)
        .padStart(2, '0'),
    )
    .join('')}`
}

const isHex = (value?: string) => !!value && /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value)

export function gridColumns(count: number): number {
  if (count <= 3) return Math.max(1, count)
  if (count === 4) return 4
  if (count <= 9) return 3
  return 4
}

export function themeVars(
  theme: Theme | undefined,
  overrides: ThemeOverrides | undefined,
  footer?: string,
): CSSProperties {
  const colors = theme?.colors ?? {
    background: '#0b1020',
    surface: '#141b33',
    ink: '#f4f6fb',
    muted: '#9aa5c4',
    accent: '#4f8cff',
    accent_ink: '#ffffff',
  }
  const accent = isHex(overrides?.accent) ? (overrides?.accent as string) : colors.accent
  const background = isHex(overrides?.background) ? (overrides?.background as string) : colors.background
  const ink = isHex(overrides?.ink) ? (overrides?.ink as string) : colors.ink
  const dark = luminance(background) < 0.4
  const accentInk = luminance(accent) > 0.5 ? '#111111' : '#ffffff'
  const fonts = theme?.fonts ?? { heading: 'Inter', body: 'Inter', mono: 'JetBrains Mono' }
  const surface = dark === (theme?.dark ?? true) ? colors.surface : dark ? mix(background, '#ffffff', 0.08) : '#ffffff'
  return {
    '--sl-bg': background,
    '--sl-surface': surface,
    '--sl-ink': ink,
    '--sl-muted': colors.muted,
    '--sl-accent': accent,
    '--sl-accent-ink': accentInk,
    '--sl-accent-soft': alpha(accent, 0.16),
    '--sl-accent-glow': alpha(accent, dark ? 0.42 : 0.26),
    '--sl-accent-2': hueShift(accent, dark ? 48 : -32),
    '--sl-accent-2-glow': alpha(hueShift(accent, dark ? 48 : -32), dark ? 0.34 : 0.2),
    '--sl-glass-a': dark ? 'rgba(255,255,255,0.09)' : 'rgba(255,255,255,0.72)',
    '--sl-glass-b': dark ? 'rgba(255,255,255,0.03)' : 'rgba(255,255,255,0.42)',
    '--sl-glass-border': dark ? 'rgba(255,255,255,0.14)' : alpha(ink, 0.1),
    '--sl-glass-hi': dark ? 'rgba(255,255,255,0.18)' : 'rgba(255,255,255,0.9)',
    '--sl-shadow': dark ? '0 30px 80px rgba(0,0,0,.45)' : '0 30px 80px rgba(20,20,40,.14)',
    '--sl-panel-fill': dark ? 'rgba(255,255,255,0.06)' : 'rgba(255,255,255,0.65)',
    '--sl-chart-0': accent,
    '--sl-chart-1': hueShift(accent, dark ? 48 : -32),
    '--sl-chart-2': hueShift(accent, 96),
    '--sl-chart-3': hueShift(accent, 150),
    '--sl-chart-4': hueShift(accent, 210),
    '--sl-chart-5': hueShift(accent, 270),
    '--sl-gradient-end': mix(background, accent, dark ? 0.38 : 0.2),
    '--sl-grid': alpha(ink, dark ? 0.045 : 0.06),
    '--sl-on-accent-muted': alpha(accentInk, 0.72),
    '--sl-on-accent-surface': alpha(accentInk, 0.12),
    '--sl-on-accent-soft': alpha(accentInk, 0.2),
    '--sl-font-heading': `'${overrides?.font_heading ?? fonts.heading}', ui-sans-serif, system-ui, sans-serif`,
    '--sl-font-body': `'${overrides?.font_body ?? fonts.body}', ui-sans-serif, system-ui, sans-serif`,
    '--sl-font-mono': `'${fonts.mono}', ui-monospace, SFMono-Regular, Menlo, monospace`,
    '--sl-footer': footer ? `'${footer.replace(/'/g, '')}'` : "''",
  } as CSSProperties
}

function Editable({
  value,
  placeholder,
  className,
  multiline,
  onCommit,
  editable,
}: {
  value: string
  placeholder: string
  className: string
  multiline?: boolean
  onCommit: (next: string) => void
  editable: boolean
}) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (ref.current && ref.current.innerText !== value && document.activeElement !== ref.current)
      ref.current.innerText = value
  }, [value])
  const commit = () => {
    const next = (ref.current?.innerText ?? '').replace(/\n+$/, '')
    if (next !== value) onCommit(next)
  }
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Enter' && !multiline) {
      event.preventDefault()
      ref.current?.blur()
    }
    if (event.key === 'Escape') {
      if (ref.current) ref.current.innerText = value
      ref.current?.blur()
    }
  }
  if (!editable) {
    return (
      <div className={`${className} sl-editable${value ? '' : ' sl-empty'}`} data-placeholder={placeholder}>
        {value}
      </div>
    )
  }
  return (
    // biome-ignore lint/a11y/useSemanticElements: inline editing on the rendered slide needs contentEditable; a textarea cannot inherit the slide typography
    <div
      ref={ref}
      className={`${className} sl-editable${value ? '' : ' sl-empty'}`}
      contentEditable
      suppressContentEditableWarning
      data-placeholder={placeholder}
      onBlur={commit}
      onKeyDown={onKeyDown}
      onClick={(event) => event.stopPropagation()}
      role="textbox"
      aria-multiline={multiline ? 'true' : undefined}
      tabIndex={0}
      aria-label={placeholder}
    >
      {value}
    </div>
  )
}

function EntryView({
  entry,
  editable,
  titleClass,
  textClass,
  onChange,
}: {
  entry: Entry
  editable: boolean
  titleClass: string
  textClass: string
  onChange: (next: Entry) => void
}) {
  return (
    <>
      <Editable
        value={entry.title}
        placeholder="Title"
        className={titleClass}
        editable={editable}
        onCommit={(title) => onChange({ ...entry, title })}
      />
      {entry.text || editable ? (
        <Editable
          value={entry.text ?? ''}
          placeholder="One line"
          className={textClass}
          multiline
          editable={editable}
          onCommit={(text) => onChange(text ? { ...entry, text } : { title: entry.title })}
        />
      ) : null}
    </>
  )
}

const GLASS = new Set(['code', 'metric', 'chart'])

function BlockView({
  block,
  selected,
  editable,
  onSelect,
  onChange,
}: {
  block: Block
  selected: boolean
  editable: boolean
  onSelect: () => void
  onChange: (patch: Partial<Block>) => void
}) {
  const common = {
    className: `sl-block sl-block-${block.type}${GLASS.has(block.type) ? ' sl-glass' : ''}${selected ? ' sl-selected' : ''}`,
    onClick: (event: { stopPropagation: () => void }) => {
      event.stopPropagation()
      onSelect()
    },
    'data-block-id': block.id,
  }
  const updateEntry = (index: number, next: Entry) => {
    const entries = [...(block.entries ?? [])]
    entries[index] = next
    onChange({ entries })
  }
  switch (block.type) {
    case 'heading':
      return (
        <div {...common}>
          <Editable
            value={block.text ?? ''}
            placeholder="Heading"
            className="sl-heading"
            onCommit={(text) => onChange({ text })}
            editable={editable}
          />
        </div>
      )
    case 'text':
      return (
        <div {...common}>
          <Editable
            value={block.text ?? ''}
            placeholder="Text"
            className="sl-text"
            multiline
            onCommit={(text) => onChange({ text })}
            editable={editable}
          />
        </div>
      )
    case 'bullets':
      return (
        <div {...common}>
          <ul className={`sl-bullets${(block.items?.length ?? 0) > 5 ? ' sl-dense' : ''}`}>
            {(block.items?.length ? block.items : ['']).map((item, index) => (
              <li key={`${block.id}-${index}`}>
                <Editable
                  value={item}
                  placeholder="Bullet"
                  className="sl-bullet"
                  onCommit={(next) => {
                    const items = [...(block.items ?? [])]
                    items[index] = next
                    onChange({ items: items.filter((entry, i) => entry.trim() || i === items.length - 1) })
                  }}
                  editable={editable}
                />
              </li>
            ))}
          </ul>
        </div>
      )
    case 'image':
      return (
        <figure {...common}>
          {block.src && /^(https?:|data:image\/)/i.test(block.src) && block.src !== 'https://' ? (
            <img src={block.src} alt={block.alt ?? ''} draggable={false} />
          ) : (
            <div className="sl-image-missing">Add an image URL in the inspector</div>
          )}
          {block.caption ? <figcaption>{block.caption}</figcaption> : null}
        </figure>
      )
    case 'code':
      return (
        <pre {...common} data-language={block.language ?? ''}>
          <code>{block.code}</code>
        </pre>
      )
    case 'quote':
      return (
        <blockquote {...common}>
          <Editable
            value={block.text ?? ''}
            placeholder="Quote"
            className="sl-quote-text"
            multiline
            onCommit={(text) => onChange({ text })}
            editable={editable}
          />
          <Editable
            value={block.attribution ?? ''}
            placeholder="Attribution"
            className="sl-quote-cite"
            onCommit={(attribution) => onChange({ attribution })}
            editable={editable}
          />
        </blockquote>
      )
    case 'metric':
      return (
        <div {...common}>
          <Editable
            value={block.value ?? ''}
            placeholder="42%"
            className="sl-metric-value"
            onCommit={(value) => onChange({ value })}
            editable={editable}
          />
          <Editable
            value={block.label ?? ''}
            placeholder="Label"
            className="sl-metric-label"
            onCommit={(label) => onChange({ label })}
            editable={editable}
          />
        </div>
      )
    case 'cards': {
      const entries = block.entries ?? []
      const dense = entries.length > 6
      return (
        <div
          {...common}
          className={`${common.className}${dense ? ' sl-dense' : ''}`}
          style={{ '--cols': gridColumns(entries.length) } as CSSProperties}
        >
          {entries.map((entry, index) => (
            <div className="sl-card sl-glass" key={`${block.id}-${index}`}>
              {entry.icon ? (
                <div className="sl-card-icon">
                  <DeckIcon name={entry.icon} />
                </div>
              ) : block.numbered ? (
                <div className="sl-card-number">{String(index + 1).padStart(2, '0')}</div>
              ) : null}
              <EntryView
                entry={entry}
                editable={editable}
                titleClass="sl-card-title"
                textClass="sl-card-text"
                onChange={(next) => updateEntry(index, next)}
              />
            </div>
          ))}
        </div>
      )
    }
    case 'steps': {
      const entries = block.entries ?? []
      return (
        <div {...common} className={`${common.className}${entries.length > 4 ? ' sl-dense' : ''}`}>
          {entries.map((entry, index) => (
            <div className="sl-step sl-glass" key={`${block.id}-${index}`}>
              <div className="sl-step-number">{entry.icon ? <DeckIcon name={entry.icon} /> : index + 1}</div>
              <EntryView
                entry={entry}
                editable={editable}
                titleClass="sl-step-title"
                textClass="sl-step-text"
                onChange={(next) => updateEntry(index, next)}
              />
            </div>
          ))}
        </div>
      )
    }
    case 'timeline': {
      const entries = block.entries ?? []
      return (
        <div {...common} style={{ '--cols': Math.max(1, entries.length) } as CSSProperties}>
          {entries.map((entry, index) => (
            <div className="sl-milestone" key={`${block.id}-${index}`}>
              <div className="sl-milestone-dot" />
              <EntryView
                entry={entry}
                editable={editable}
                titleClass="sl-milestone-title"
                textClass="sl-milestone-text"
                onChange={(next) => updateEntry(index, next)}
              />
            </div>
          ))}
        </div>
      )
    }
    case 'chart':
      return (
        <figure {...common}>
          {block.title ? <figcaption className="sl-chart-title">{block.title}</figcaption> : null}
          <Chart kind={block.kind ?? 'bar'} series={block.series ?? []} unit={block.unit} title={block.title} />
        </figure>
      )
    default:
      return null
  }
}

function splitColumns(blocks: Block[]): [Block[], Block[]] {
  const left: Block[] = []
  const right: Block[] = []
  blocks.forEach((block, index) => {
    ;((block.column ?? (index % 2 === 0 ? 'left' : 'right')) === 'left' ? left : right).push(block)
  })
  return [left, right]
}

export interface SlideCanvasProps {
  slide: Slide
  index: number
  total: number
  theme?: Theme
  overrides?: ThemeOverrides
  footer?: string
  author?: string
  editable?: boolean
  animate?: boolean
  selectedBlockId?: string | null
  onSelectBlock?: (blockId: string | null) => void
  onChangeSlide?: (patch: Partial<Slide>) => void
  onChangeBlock?: (blockId: string, patch: Partial<Block>) => void
  className?: string
}

export function SlideCanvas({
  slide,
  index,
  total,
  theme,
  overrides,
  footer,
  author,
  editable = false,
  animate = false,
  selectedBlockId,
  onSelectBlock,
  onChangeSlide,
  onChangeBlock,
  className,
}: SlideCanvasProps) {
  const frame = useRef<HTMLDivElement>(null)
  const [scale, setScale] = useState(0.5)
  useEffect(() => {
    const node = frame.current
    if (!node) return
    const observer = new ResizeObserver(([entry]) => setScale(entry.contentRect.width / CANVAS_W))
    observer.observe(node)
    return () => observer.disconnect()
  }, [])

  const patchSlide = (patch: Partial<Slide>) => onChangeSlide?.(patch)
  const patchBlock = (id: string, patch: Partial<Block>) => onChangeBlock?.(id, patch)
  const blocks = (list: Block[]) =>
    list.map((block) => (
      <BlockView
        key={block.id}
        block={block}
        selected={selectedBlockId === block.id}
        editable={editable}
        onSelect={() => onSelectBlock?.(block.id)}
        onChange={(patch) => patchBlock(block.id, patch)}
      />
    ))
  const kicker =
    slide.kicker || editable ? (
      <Editable
        value={slide.kicker ?? ''}
        placeholder="Kicker"
        className="sl-kicker"
        onCommit={(value) => patchSlide({ kicker: value })}
        editable={editable}
      />
    ) : null
  const title = (cls = 'sl-title') =>
    slide.layout === 'blank' ? null : (
      <Editable
        value={slide.title ?? ''}
        placeholder="Slide title"
        className={cls}
        onCommit={(value) => patchSlide({ title: value })}
        editable={editable}
      />
    )
  const subtitle =
    slide.layout === 'blank' || (!slide.subtitle && !editable) ? null : (
      <Editable
        value={slide.subtitle ?? ''}
        placeholder="Subtitle"
        className="sl-subtitle"
        onCommit={(value) => patchSlide({ subtitle: value })}
        editable={editable}
      />
    )

  let body: ReactElement
  if (slide.layout === 'title') {
    body = (
      <div className="sl-stack sl-center sl-hero">
        {kicker}
        {title('sl-deck-title')}
        {subtitle}
        <div className="sl-hero-meta">
          <div className="sl-accent-bar" />
          {author ? <span>{author}</span> : null}
        </div>
        <div className="sl-blocks">{blocks(slide.blocks)}</div>
      </div>
    )
  } else if (slide.layout === 'section') {
    body = (
      <>
        <div className="sl-section-decor" aria-hidden="true">
          {String(index + 1).padStart(2, '0')}
        </div>
        <div className="sl-stack sl-section">
          {kicker}
          {title('sl-title sl-title-large')}
          {subtitle}
          <div className="sl-blocks">{blocks(slide.blocks)}</div>
        </div>
      </>
    )
  } else if (slide.layout === 'statement') {
    body = (
      <div className="sl-stack sl-center sl-statement">
        {kicker}
        {title('sl-title sl-title-statement')}
        {subtitle}
        <div className="sl-blocks">{blocks(slide.blocks)}</div>
      </div>
    )
  } else if (slide.layout === 'image') {
    const image = slide.blocks.find((block) => block.type === 'image')
    const src = image?.src && /^(https?:|data:image\/)/i.test(image.src) && image.src !== 'https://' ? image.src : ''
    body = (
      <div className="sl-stack sl-image-layout">
        {src ? (
          <img className="sl-image-full" src={src} alt={image?.alt ?? ''} draggable={false} />
        ) : (
          <div className="sl-image-full sl-image-missing">Add an image block with a URL</div>
        )}
        <div className="sl-image-overlay">
          {kicker}
          {title()}
          {subtitle}
          <div className="sl-blocks">{blocks(slide.blocks.filter((block) => block !== image))}</div>
        </div>
      </div>
    )
  } else if (slide.layout === 'split') {
    body = (
      <div className="sl-split">
        <div className="sl-split-copy">
          {kicker}
          {title('sl-title sl-title-split')}
          {subtitle}
        </div>
        <div className="sl-split-panel sl-glass">
          <div className="sl-blocks">{blocks(slide.blocks)}</div>
        </div>
      </div>
    )
  } else if (slide.layout === 'two-column' || slide.blocks.some((block) => block.column)) {
    const [left, right] = splitColumns(slide.blocks)
    body = (
      <div className="sl-stack">
        <div className="sl-head">
          {kicker}
          {title()}
          {subtitle}
        </div>
        <div className="sl-columns">
          <div className="sl-column">{blocks(left)}</div>
          <div className="sl-column">{blocks(right)}</div>
        </div>
      </div>
    )
  } else {
    body = (
      <div className="sl-stack">
        {slide.layout === 'blank' ? null : (
          <div className="sl-head">
            {kicker}
            {title()}
            {subtitle}
          </div>
        )}
        <div className="sl-blocks">{blocks(slide.blocks)}</div>
      </div>
    )
  }

  const background = isHex(slide.background)
    ? ({ '--sl-bg': slide.background } as CSSProperties)
    : slide.background && /^https?:/i.test(slide.background)
      ? { backgroundImage: `url('${slide.background.replace(/'/g, '')}')` }
      : undefined

  return (
    <div
      ref={frame}
      className={`sl-frame${className ? ` ${className}` : ''}`}
      style={themeVars(theme, overrides, footer)}
    >
      <div className="sl-scaler" style={{ transform: `scale(${scale})` }}>
        <section
          key={animate ? slide.id : undefined}
          className={`sl-slide sl-layout-${slide.layout} sl-variant-${slide.variant ?? 'default'}${animate ? ' sl-animate' : ''}`}
          style={background}
          onClick={() => onSelectBlock?.(null)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') onSelectBlock?.(null)
          }}
          role={editable ? 'group' : undefined}
          aria-label={`Slide ${index + 1}`}
        >
          <div className="sl-mesh" aria-hidden="true">
            <i className="sl-blob sl-blob-a" />
            <i className="sl-blob sl-blob-b" />
            <i className="sl-blob sl-blob-c" />
          </div>
          <Motif visual={slide.visual} />
          {body}
          <footer className="sl-footer">
            <span className="sl-footer-text" />
            <span>
              {index + 1} / {total}
            </span>
          </footer>
        </section>
      </div>
    </div>
  )
}
