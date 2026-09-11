import { type CSSProperties, type KeyboardEvent, type ReactElement, useEffect, useRef, useState } from 'react'
import type { Block, Entry, Slide, Theme, ThemeOverrides } from './types'

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
    '--sl-accent-glow': alpha(accent, dark ? 0.35 : 0.22),
    '--sl-gradient-end': mix(background, accent, dark ? 0.35 : 0.18),
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
    className: `sl-block sl-block-${block.type}${selected ? ' sl-selected' : ''}`,
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
            <div className="sl-card" key={`${block.id}-${index}`}>
              {block.numbered ? <div className="sl-card-number">{String(index + 1).padStart(2, '0')}</div> : null}
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
            <div className="sl-step" key={`${block.id}-${index}`}>
              <div className="sl-step-number">{index + 1}</div>
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
      <div className="sl-stack sl-section">
        <div className="sl-section-number">{String(index + 1).padStart(2, '0')}</div>
        {kicker}
        {title('sl-title sl-title-large')}
        {subtitle}
        <div className="sl-blocks">{blocks(slide.blocks)}</div>
      </div>
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
          className={`sl-slide sl-layout-${slide.layout} sl-variant-${slide.variant ?? 'default'}`}
          style={background}
          onClick={() => onSelectBlock?.(null)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') onSelectBlock?.(null)
          }}
          role={editable ? 'group' : undefined}
          aria-label={`Slide ${index + 1}`}
        >
          <div className="sl-orb sl-orb-a" />
          <div className="sl-orb sl-orb-b" />
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
