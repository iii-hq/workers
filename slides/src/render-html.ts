import { iconSvg } from './deck-icons.js'
import type { Block, Deck, Entry, Slide, ThemeOverrides } from './model.js'
import { diagramSvg } from './render-diagrams.js'
import { chartSvg, motifSvg } from './render-visuals.js'
import { googleFontsHref, hueShift, mix, type ResolvedTheme, resolveTheme, withAlpha } from './themes.js'

export const SLIDE_WIDTH = 1600
export const SLIDE_HEIGHT = 900

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function inline(value: string): string {
  return escapeHtml(value)
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<em>$2</em>')
}

function safeUrl(value: string): string {
  const trimmed = value.trim()
  if (/^(https?:|data:image\/|\/|\.\/|\.\.\/)/i.test(trimmed)) return escapeHtml(trimmed)
  return ''
}

export function gridColumns(count: number): number {
  if (count <= 3) return Math.max(1, count)
  if (count === 4) return 4
  if (count <= 9) return 3
  return 4
}

class Reveal {
  index = 0
  next(): string {
    const attr = ` class="rv" style="--i:${this.index}"`
    this.index += 1
    return attr
  }
  wrap(className: string): string {
    const attr = ` class="${className} rv" style="--i:${this.index}"`
    this.index += 1
    return attr
  }
}

function entries(list: Entry[], render: (entry: Entry, index: number) => string): string {
  return list.map(render).join('')
}

export function renderBlock(block: Block, rv = new Reveal()): string {
  return renderBlockInner(block, rv).replace(/^<([a-z0-9]+)/, `<$1 data-block-id="${escapeHtml(block.id)}"`)
}

function renderBlockInner(block: Block, rv: Reveal): string {
  switch (block.type) {
    case 'heading':
      return `<h3${rv.wrap('block block-heading')}>${inline(block.text)}</h3>`
    case 'text':
      return `<p${rv.wrap('block block-text')}>${inline(block.text)}</p>`
    case 'bullets':
      return `<ul class="block block-bullets${block.items.length > 5 ? ' dense' : ''}">${block.items.map((item) => `<li${rv.next()}>${inline(item)}</li>`).join('')}</ul>`
    case 'image': {
      const src = safeUrl(block.src)
      const img = src
        ? `<img src="${src}" alt="${escapeHtml(block.alt ?? '')}" loading="lazy" />`
        : `<div class="image-missing">Image unavailable</div>`
      return `<figure${rv.wrap('block block-image')}>${img}${block.caption ? `<figcaption>${inline(block.caption)}</figcaption>` : ''}</figure>`
    }
    case 'code':
      return `<pre${rv.wrap('block block-code glass')}${block.language ? ` data-language="${escapeHtml(block.language)}"` : ''}><code>${escapeHtml(block.code)}</code></pre>`
    case 'quote':
      return `<blockquote${rv.wrap('block block-quote')}><p>${inline(block.text)}</p>${block.attribution ? `<cite>${inline(block.attribution)}</cite>` : ''}</blockquote>`
    case 'metric':
      return `<div${rv.wrap('block block-metric glass')}><div class="metric-value">${inline(block.value)}</div><div class="metric-label">${inline(block.label)}</div></div>`
    case 'cards': {
      const cols = gridColumns(block.entries.length)
      const dense = block.entries.length > 6 ? ' dense' : ''
      return `<div class="block block-cards${dense}" style="--cols:${cols}">${entries(
        block.entries,
        (entry, index) =>
          `<div${rv.wrap('card glass')}>${entry.icon ? `<div class="card-icon">${iconSvg(entry.icon)}</div>` : ''}${block.numbered || !entry.icon ? `<div class="card-number">${String(index + 1).padStart(2, '0')}</div>` : ''}<div class="card-title">${inline(entry.title)}</div>${entry.text ? `<div class="card-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    }
    case 'steps':
      return `<div class="block block-steps${block.entries.length > 4 ? ' dense' : ''}">${entries(
        block.entries,
        (entry, index) =>
          `<div${rv.wrap('step glass')}><div class="step-number">${String(index + 1).padStart(2, '0')}${entry.icon ? iconSvg(entry.icon) : ''}</div><div class="step-title">${inline(entry.title)}</div>${entry.text ? `<div class="step-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    case 'timeline':
      return `<div class="block block-timeline" style="--cols:${Math.max(1, block.entries.length)}">${entries(
        block.entries,
        (entry) =>
          `<div${rv.wrap('milestone')}><div class="milestone-dot"></div><div class="milestone-title">${inline(entry.title)}</div>${entry.text ? `<div class="milestone-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    case 'chart':
      return `<figure${rv.wrap('block block-chart glass')}>${block.title ? `<figcaption class="chart-title">${inline(block.title)}</figcaption>` : ''}${chartSvg(block.kind, block.series, block.unit, block.title, block.log)}</figure>`
    case 'diagram':
      return `<figure${rv.wrap('block block-diagram')}>${block.title ? `<figcaption class="diagram-title">${inline(block.title)}</figcaption>` : ''}${diagramSvg(block, block.title)}</figure>`
    case 'table':
      return `<table${rv.wrap(`block block-table${block.rows.length > 5 ? ' dense' : ''}`)}>${block.columns.length ? `<thead><tr>${block.columns.map((cell) => `<th>${inline(cell)}</th>`).join('')}</tr></thead>` : ''}<tbody>${block.rows.map((row) => `<tr>${row.map((cell) => `<td>${inline(cell)}</td>`).join('')}</tr>`).join('')}</tbody></table>`
    default:
      return ''
  }
}

function splitColumns(blocks: Block[]): [Block[], Block[]] {
  const left: Block[] = []
  const right: Block[] = []
  for (const [index, block] of blocks.entries()) {
    ;((block.column ?? (index % 2 === 0 ? 'left' : 'right')) === 'left' ? left : right).push(block)
  }
  return [left, right]
}

export function renderSlideBody(slide: Slide, index: number, deck: Pick<Deck, 'author'>): string {
  const rv = new Reveal()
  const kicker = ''
  const title = slide.title ? `<h2${rv.wrap('slide-title')}>${inline(slide.title)}</h2>` : ''
  const subtitle = slide.subtitle ? `<p${rv.wrap('slide-subtitle')}>${inline(slide.subtitle)}</p>` : ''
  const blocks = () => slide.blocks.map((block) => renderBlock(block, rv)).join('')
  const columns = () => {
    const [left, right] = splitColumns(slide.blocks)
    return `<div class="columns"><div class="column">${left.map((block) => renderBlock(block, rv)).join('')}</div><div class="column">${right.map((block) => renderBlock(block, rv)).join('')}</div></div>`
  }
  switch (slide.layout) {
    case 'title':
      return `<div class="stack center hero"><div${rv.wrap('accent-bar')}></div>${slide.title ? `<h1${rv.wrap('deck-title')}>${inline(slide.title)}</h1>` : ''}${subtitle}${deck.author ? `<div${rv.wrap('hero-meta')}><span>${inline(deck.author)}</span></div>` : ''}<div class="blocks">${blocks()}</div></div>`
    case 'section':
      return `<div class="section-decor" aria-hidden="true">${String(index + 1).padStart(2, '0')}</div><div class="stack section">${kicker}${title}${subtitle}<div class="blocks">${blocks()}</div></div>`
    case 'statement':
      return `<div class="stack center statement">${kicker}${title}${subtitle}<div class="blocks">${blocks()}</div></div>`
    case 'split':
      return `<div class="split"><div class="split-copy">${kicker}${title}${subtitle}</div><div${rv.wrap('split-panel glass')}><div class="blocks">${blocks()}</div></div></div>`
    case 'image': {
      const image = slide.blocks.find((block) => block.type === 'image')
      const rest = slide.blocks
        .filter((block) => block !== image)
        .map((block) => renderBlock(block, rv))
        .join('')
      const src = image && image.type === 'image' ? safeUrl(image.src) : ''
      return `<div class="stack image-layout">${src ? `<img class="image-full" src="${src}" alt="${escapeHtml(image && image.type === 'image' ? (image.alt ?? '') : '')}" />` : ''}<div class="image-overlay">${kicker}${title}${subtitle}<div class="blocks">${rest}</div></div></div>`
    }
    case 'two-column':
      return `<div class="stack"><div class="head">${kicker}${title}${subtitle}</div>${columns()}</div>`
    case 'blank':
      return `<div class="stack"><div class="blocks">${blocks()}</div></div>`
    default:
      if (slide.blocks.some((block) => block.column))
        return `<div class="stack"><div class="head">${kicker}${title}${subtitle}</div>${columns()}</div>`
      return `<div class="stack"><div class="head">${kicker}${title}${subtitle}</div><div class="blocks">${blocks()}</div></div>`
  }
}

export function renderSlide(
  slide: Slide,
  index: number,
  total: number,
  theme: ResolvedTheme,
  deck: Pick<Deck, 'author' | 'title'>,
): string {
  const background = slide.background
    ? /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(slide.background)
      ? ` style="--bg:${escapeHtml(slide.background)}"`
      : safeUrl(slide.background)
        ? ` style="background-image:url('${safeUrl(slide.background)}')"`
        : ''
    : ''
  const meta = `<header class="meta"><span class="meta-kicker">${slide.kicker ? inline(slide.kicker) : ''}</span><span class="meta-title">${inline(deck.title)}</span><span class="meta-index">${String(index + 1).padStart(2, '0')} <em>/ ${String(total).padStart(2, '0')}</em></span></header>`
  const footer = `<footer class="slide-footer"><span>${theme.footer ? inline(theme.footer) : deck.author ? inline(deck.author) : ''}</span><span class="slide-number">${index + 1}</span></footer>`
  const notes = slide.notes ? `<aside class="notes">${inline(slide.notes)}</aside>` : ''
  const variant = slide.variant ?? 'default'
  return `<section class="slide layout-${slide.layout} variant-${variant}" data-index="${index}" id="slide-${index + 1}"${background}>${STARFIELD}${GRAIN}${motifSvg(slide.visual)}${meta}<div class="body">${renderSlideBody(slide, index, deck)}</div>${footer}${notes}</section>`
}

function starfield(): string {
  let seed = 7
  const rand = () => {
    seed = (seed * 16807) % 2147483647
    return seed / 2147483647
  }
  const dots = Array.from({ length: 170 }, () => {
    const x = (rand() * SLIDE_WIDTH * 1.12).toFixed(0)
    const y = (rand() * SLIDE_HEIGHT * 1.12).toFixed(0)
    const r = (0.5 + rand() * 1.3).toFixed(2)
    const o = (0.25 + rand() * 0.7).toFixed(2)
    return `<circle cx="${x}" cy="${y}" r="${r}" opacity="${o}"/>`
  }).join('')
  return `<svg class="stars" viewBox="0 0 ${Math.round(SLIDE_WIDTH * 1.12)} ${Math.round(SLIDE_HEIGHT * 1.12)}" fill="currentColor" aria-hidden="true">${dots}</svg>`
}

const STARFIELD = starfield()
const GRAIN = `<svg class="grain" aria-hidden="true"><filter id="g"><feTurbulence type="fractalNoise" baseFrequency=".9" numOctaves="2" stitchTiles="stitch"/><feColorMatrix values="0 0 0 0 .5 0 0 0 0 .5 0 0 0 0 .5 0 0 0 .9 0"/></filter><rect width="100%" height="100%" filter="url(#g)"/></svg>`

export function deckCss(theme: ResolvedTheme): string {
  const c = theme.colors
  const dark = theme.dark
  const hair = withAlpha(c.ink, dark ? 0.12 : 0.14)
  const hairStrong = withAlpha(c.ink, dark ? 0.24 : 0.28)
  const card = dark ? withAlpha('#ffffff', 0.025) : withAlpha('#000000', 0.02)
  const accent2 = mix(c.accent, c.ink, 0.45)
  return `
:root{--bg:${c.background};--surface:${c.surface};--ink:${c.ink};--muted:${c.muted};--accent:${c.accent};--accent-2:${accent2};--accent-ink:${c.accent_ink};--accent-soft:${withAlpha(c.accent, 0.14)};--accent-glow:${withAlpha(c.accent, dark ? 0.22 : 0.16)};--hair:${hair};--hair-strong:${hairStrong};--card:${card};--font-heading:'${theme.fonts.heading}',Georgia,'Times New Roman',serif;--font-body:'${theme.fonts.body}',ui-sans-serif,system-ui,sans-serif;--font-mono:'${theme.fonts.mono}',ui-monospace,SFMono-Regular,Menlo,monospace;--radius:${theme.radius}px;--heading-weight:${theme.heading_weight};--w:${SLIDE_WIDTH}px;--h:${SLIDE_HEIGHT}px;--ease:cubic-bezier(.22,.61,.36,1)}
*{box-sizing:border-box}
html,body{margin:0;background:#000;color:var(--ink);font-family:var(--font-body);height:100%;-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility}
body.deck{overflow:hidden}
.stage{position:fixed;inset:0;overflow:hidden}
.frame{position:absolute;left:50%;top:50%;width:var(--w);height:var(--h);transform-origin:center center;transform:translate(-50%,-50%)}
.slide{position:absolute;inset:0;width:var(--w);height:var(--h);background:var(--bg);color:var(--ink);padding:64px 96px 56px;overflow:hidden;background-size:cover;background-position:center;display:flex;flex-direction:column;isolation:isolate;opacity:0;visibility:hidden;pointer-events:none;will-change:opacity,transform}
.slide.active{opacity:1;visibility:visible;pointer-events:auto;z-index:2}
.slide.leaving{visibility:visible;z-index:1}
.t-fade .slide{transition:opacity .8s var(--ease),transform 1s var(--ease);transform:scale(1.015)}
.t-fade .slide.active{transform:scale(1)}
.t-fade .slide.leaving{opacity:0;transform:scale(.995)}
.t-slide .slide{transition:opacity .6s var(--ease),transform .8s var(--ease);transform:translateX(6%)}
.t-slide .slide.active{transform:translateX(0)}
.t-slide .slide.leaving{opacity:0;transform:translateX(-6%)}
.t-slide.backward .slide{transform:translateX(-6%)}
.t-slide.backward .slide.leaving{transform:translateX(6%)}
.t-zoom .slide{transition:opacity .6s var(--ease),transform .9s var(--ease);transform:scale(.9)}
.t-zoom .slide.active{transform:scale(1)}
.t-zoom .slide.leaving{opacity:0;transform:scale(1.08)}
.t-none .slide{transition:none}
.stars{position:absolute;inset:-6%;width:112%;height:112%;z-index:-3;pointer-events:none;opacity:${dark ? 0.55 : 0.18};animation:starDrift 90s var(--ease) infinite alternate}
@keyframes starDrift{to{transform:translate(-2.5%,1.5%)}}
.grain{position:absolute;inset:0;z-index:-2;pointer-events:none;opacity:${dark ? 0.07 : 0.05};mix-blend-mode:overlay}
.mesh{display:none}
.variant-gradient{background:radial-gradient(ellipse 70% 60% at 12% 100%,var(--accent-glow),transparent 60%),var(--bg)}
.variant-muted{--bg:var(--surface)}
.variant-accent{--bg:var(--accent);--ink:var(--accent-ink);--muted:${withAlpha(c.accent_ink, 0.7)};--hair:${withAlpha(c.accent_ink, 0.2)};--hair-strong:${withAlpha(c.accent_ink, 0.4)};--card:${withAlpha(c.accent_ink, 0.06)};--accent:var(--accent-ink);--accent-soft:${withAlpha(c.accent_ink, 0.12)}}
.variant-accent .stars{opacity:.15}
.meta{display:grid;grid-template-columns:1fr auto 1fr;align-items:center;gap:32px;padding-bottom:22px;border-bottom:1px solid var(--hair);font-family:var(--font-mono);font-size:14px;letter-spacing:.2em;text-transform:uppercase;color:var(--muted);flex:none}
.meta-kicker{display:inline-flex;align-items:center;gap:14px;color:var(--ink)}
.meta-kicker::before{content:'';width:9px;height:9px;background:var(--accent);flex:none}
.meta-title{text-align:center;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:560px}
.meta-index{text-align:right;color:var(--ink)}
.meta-index em{font-style:normal;color:var(--muted)}
.body{display:flex;flex-direction:column;flex:1;min-height:0;padding-top:48px;padding-bottom:40px}
.stack{display:flex;flex-direction:column;gap:28px;flex:1;min-height:0}
.head{display:flex;flex-direction:column;gap:18px;padding-bottom:36px;max-width:1240px}
.stack.center{justify-content:center;align-items:flex-start}
.hero{justify-content:flex-end;padding-bottom:16px;max-width:1120px}
.hero .accent-bar{width:64px;height:1px;background:var(--accent);margin-bottom:8px}
.deck-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:136px;line-height:.96;letter-spacing:-0.025em;margin:0;text-wrap:balance}
.deck-title em,.slide-title em{font-style:italic;color:var(--accent);font-weight:inherit}
.hero-meta{display:flex;align-items:center;gap:20px;color:var(--muted);font-family:var(--font-mono);font-size:14px;letter-spacing:.2em;text-transform:uppercase;margin-top:24px}
.hero-meta .accent-bar{display:none}
.slide-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:72px;line-height:1.02;letter-spacing:-0.02em;margin:0;text-wrap:balance}
.slide-subtitle{font-size:27px;line-height:1.45;color:var(--muted);margin:0;max-width:1040px;font-weight:300}
.stack.statement{align-items:center;text-align:center;max-width:none;justify-content:center;gap:40px}
.stack.statement .slide-title{font-size:100px;line-height:1;max-width:1360px;font-style:italic;letter-spacing:-0.025em}
.stack.statement .slide-subtitle{text-align:center;max-width:1100px;font-size:25px}
.stack.statement::before,.stack.statement::after{content:'';width:120px;height:1px;background:var(--hair-strong)}
.section{justify-content:center;max-width:1000px;gap:28px}
.section .slide-title{font-size:104px;letter-spacing:-0.03em}
.section-decor{position:absolute;right:56px;bottom:-30px;font-family:var(--font-heading);font-size:640px;line-height:1;letter-spacing:-0.06em;color:transparent;-webkit-text-stroke:1px var(--hair-strong);z-index:-1;pointer-events:none;user-select:none}
.blocks{display:flex;flex-direction:column;gap:28px;min-height:0;flex:1;justify-content:flex-start}
.blocks:empty{display:none}
.blocks > .block-steps:not(:last-child),.blocks > .block-cards:not(:last-child){flex:0 0 auto}
.hero .blocks,.section .blocks,.statement .blocks{flex:0 0 auto}
.columns{display:grid;grid-template-columns:1fr 1fr;gap:80px;flex:1;min-height:0;align-content:center}
.column{display:flex;flex-direction:column;gap:28px;min-width:0;justify-content:center}
.column + .column{border-left:1px solid var(--hair);padding-left:80px;margin-left:-40px}
.split{display:grid;grid-template-columns:1fr 1fr;gap:88px;flex:1;min-height:0;align-items:center}
.split-copy{display:flex;flex-direction:column;gap:22px;min-width:0}
.split-copy .slide-title{font-size:74px}
.split-panel{display:flex;flex-direction:column;min-height:0;border-left:1px solid var(--hair);padding-left:64px;align-self:stretch;justify-content:center}
.split-panel .blocks{gap:20px;flex:0 0 auto}
.split-panel .block-cards{grid-template-columns:repeat(2,minmax(0,1fr))}
.block{margin:0}
.block-heading{font-family:var(--font-heading);font-size:40px;font-weight:var(--heading-weight);line-height:1.15;letter-spacing:-0.015em}
.block-text{font-size:27px;line-height:1.5;max-width:1100px;color:var(--ink);font-weight:300}
.blocks > .block-text:not(:first-child){border-top:1px solid var(--hair);padding-top:26px;font-family:var(--font-heading);font-size:34px;line-height:1.3;color:var(--muted);font-weight:400}
.blocks > .block-text:not(:first-child) strong{color:var(--ink);font-weight:400}
.block-bullets{font-size:27px;line-height:1.4;padding-left:0;list-style:none;display:flex;flex-direction:column;gap:0;counter-reset:b;font-weight:300}
.block-bullets.dense{font-size:24px}
.block-bullets li{position:relative;padding:18px 0 18px 64px;border-top:1px solid var(--hair);counter-increment:b}
.block-bullets li:last-child{border-bottom:1px solid var(--hair)}
.block-bullets li::before{content:counter(b,decimal-leading-zero);position:absolute;left:0;top:22px;font-family:var(--font-mono);font-size:15px;letter-spacing:.1em;color:var(--accent)}
.block-image{display:flex;flex-direction:column;gap:12px;align-items:flex-start;min-height:0;flex:1}
.block-image img{max-width:100%;max-height:100%;object-fit:contain;border-radius:var(--radius);border:1px solid var(--hair)}
.block-image figcaption{font-size:16px;color:var(--muted);font-family:var(--font-mono);letter-spacing:.08em;text-transform:uppercase}
.image-missing{padding:48px;border:1px dashed var(--hair-strong);border-radius:var(--radius);color:var(--muted);font-size:22px}
.block-code{background:var(--card);border:1px solid var(--hair);border-radius:var(--radius);padding:30px 36px;font-family:var(--font-mono);font-size:21px;line-height:1.6;overflow:hidden;position:relative;white-space:pre-wrap;word-break:break-word}
.block-code[data-language]::before{content:attr(data-language);position:absolute;top:14px;right:20px;font-size:12px;color:var(--muted);text-transform:uppercase;letter-spacing:.2em}
.block-quote{position:relative;padding:8px 0 8px 40px;display:flex;flex-direction:column;gap:20px;border-left:1px solid var(--accent)}
.block-quote p{margin:0;font-family:var(--font-heading);font-size:52px;line-height:1.2;font-style:italic;text-wrap:balance;letter-spacing:-0.015em}
.block-quote cite{font-style:normal;font-size:14px;color:var(--muted);font-family:var(--font-mono);letter-spacing:.2em;text-transform:uppercase}
.statement .block-quote{border-left:0;padding:0;align-items:center}
.statement .block-quote p{font-size:64px}
.block-metric{display:flex;flex-direction:column;gap:8px;align-self:flex-start;min-width:0}
.column .block-metric{padding:22px 0;border-top:1px solid var(--hair)}
.column .block-metric:last-child{border-bottom:1px solid var(--hair)}
.metric-value{font-family:var(--font-heading);font-size:120px;line-height:.95;font-weight:var(--heading-weight);letter-spacing:-0.035em;color:var(--accent)}
.metric-label{font-size:14px;color:var(--muted);font-family:var(--font-mono);letter-spacing:.2em;text-transform:uppercase}
.block-cards{display:grid;grid-template-columns:repeat(var(--cols,3),minmax(0,1fr));gap:16px;min-height:0;align-content:center}
.card{display:flex;flex-direction:column;gap:14px;padding:30px 30px 28px;min-width:0;position:relative;background:var(--card);border:1px solid var(--hair);border-radius:var(--radius)}
.card::before,.card::after{content:'';position:absolute;width:12px;height:12px;border-color:var(--accent);border-style:solid}
.card::before{left:-1px;top:-1px;border-width:1px 0 0 1px}
.card::after{right:-1px;bottom:-1px;border-width:0 1px 1px 0}
.card-icon{width:34px;height:34px;color:var(--accent);margin-bottom:8px}
.card-icon .icon{width:34px;height:34px;stroke-width:1.4}
.card-number{position:absolute;top:26px;right:28px;font-family:var(--font-mono);font-size:13px;letter-spacing:.16em;color:var(--muted)}
.card-title{font-family:var(--font-heading);font-size:34px;font-weight:var(--heading-weight);line-height:1.1;text-wrap:balance;letter-spacing:-0.015em}
.card-text{font-size:19px;line-height:1.45;color:var(--muted);font-weight:300}
.block-cards.dense{gap:14px}
.block-cards.dense .card{padding:22px 24px 22px;gap:10px}
.block-cards.dense .card-icon{width:28px;height:28px;margin-bottom:2px}
.block-cards.dense .card-icon .icon{width:28px;height:28px}
.block-cards.dense .card-title{font-size:28px}
.block-cards.dense .card-text{font-size:17px}
.block-steps{display:grid;grid-auto-flow:column;grid-auto-columns:minmax(0,1fr);gap:40px;min-height:0;position:relative;align-self:stretch;flex:1}
.step{display:flex;flex-direction:column;gap:16px;padding-top:28px;position:relative;border-top:1px solid var(--hair-strong);min-width:0}
.step::before{content:'';position:absolute;left:0;top:-4px;width:7px;height:7px;background:var(--accent);border-radius:50%}
.step-number{font-family:var(--font-mono);font-size:14px;letter-spacing:.2em;color:var(--accent);display:flex;align-items:center;justify-content:space-between;height:40px;padding-right:8px}
.step-number .icon{width:36px;height:36px;stroke-width:1.3;color:var(--ink)}
.step-title{font-family:var(--font-heading);font-size:46px;font-weight:var(--heading-weight);line-height:1.02;text-wrap:balance;letter-spacing:-0.02em;margin-top:6px}
.step-text{font-size:22px;line-height:1.45;color:var(--muted);font-weight:300}
.block-steps.dense{gap:28px}
.block-steps.dense .step-title{font-size:32px}
.block-steps.dense .step-text{font-size:18px}
.block-timeline{display:grid;grid-template-columns:repeat(var(--cols,4),minmax(0,1fr));gap:40px;position:relative;padding-top:44px;align-self:stretch;width:100%}
.block-timeline::before{content:'';position:absolute;left:0;right:0;top:12px;height:1px;background:var(--hair-strong)}
.milestone{display:flex;flex-direction:column;gap:12px;position:relative;min-width:0}
.milestone-dot{position:absolute;top:-38px;left:0;width:13px;height:13px;border-radius:50%;background:var(--bg);border:1px solid var(--accent);box-sizing:border-box}
.milestone:first-child .milestone-dot{background:var(--accent)}
.milestone-title{font-family:var(--font-heading);font-size:38px;font-weight:var(--heading-weight);line-height:1.1;letter-spacing:-0.015em}
.milestone-text{font-size:20px;line-height:1.45;color:var(--muted);font-weight:300}
.image-layout{position:relative;margin:-48px -96px -56px;flex:1}
.image-full{position:absolute;inset:0;width:100%;height:100%;object-fit:cover}
.image-overlay{position:absolute;left:0;right:0;bottom:0;padding:72px 96px 96px;background:linear-gradient(to top,rgba(0,0,0,.82),rgba(0,0,0,0));color:#fff;display:flex;flex-direction:column;gap:16px}
.image-overlay .slide-subtitle{color:rgba(255,255,255,.8)}
.slide-footer{position:absolute;left:96px;right:96px;bottom:26px;display:flex;justify-content:space-between;font-family:var(--font-mono);font-size:12px;color:var(--muted);letter-spacing:.2em;text-transform:uppercase}
.slide-footer .slide-number{display:none}
.card-icon,.step-number,.card-number{font-weight:400}
.block-chart{padding:26px 30px 18px;display:flex;flex-direction:column;gap:12px;flex:1;min-height:0;background:var(--card);border:1px solid var(--hair);border-radius:var(--radius)}
.block-diagram{display:flex;flex-direction:column;gap:10px;flex:1;min-height:0}
.diagram-title{font-family:var(--font-mono);font-size:13px;letter-spacing:.2em;text-transform:uppercase;color:var(--muted)}
.diagram{width:100%;height:100%;min-height:0;flex:1;overflow:visible;color:var(--accent)}
.diagram text{font-family:var(--font-body);fill:var(--ink);font-weight:300}
.diagram .guide{fill:none;stroke:var(--hair-strong);stroke-width:1}
.diagram .guide.dashed{stroke-dasharray:4 8;stroke:var(--hair)}
.diagram .dot{fill:var(--accent);stroke:var(--bg);stroke-width:3}
.diagram .halo{fill:var(--accent);opacity:.12}
.diagram .edge{fill:none;stroke:var(--accent);stroke-width:1.2;opacity:.45}
.diagram .label{font-size:19px;fill:var(--ink)}
.diagram .label-strong{font-family:var(--font-heading);font-size:26px;letter-spacing:-0.01em}
.diagram .hub rect{fill:var(--card);stroke:var(--hair)}
.diagram .hub-label{font-family:var(--font-heading);font-size:26px;letter-spacing:-0.01em}
.diagram .sub{font-family:var(--font-mono);font-size:12px;letter-spacing:.14em;text-transform:uppercase;fill:var(--muted)}
.diagram .center{font-family:var(--font-heading);font-size:30px;fill:var(--ink);letter-spacing:-0.01em}
.diagram .quadrant{font-family:var(--font-mono);font-size:12px;letter-spacing:.2em;text-transform:uppercase;fill:var(--muted)}
.diagram .axis{font-family:var(--font-mono);font-size:13px;letter-spacing:.2em;text-transform:uppercase;fill:var(--ink)}
.diagram .tick{font-family:var(--font-mono);font-size:12px;letter-spacing:.12em;text-transform:uppercase;fill:var(--muted)}
.diagram .point-label{font-family:var(--font-heading);font-size:26px;letter-spacing:-0.01em}
.diagram .shape{fill:var(--accent);fill-opacity:.14;stroke:var(--accent);stroke-width:1.5}
.diagram .arc{fill:none;stroke:var(--accent);stroke-width:1.5}
.diagram .arrow{fill:none;stroke:var(--accent);stroke-width:1.5}
.diagram .index{font-family:var(--font-mono);font-size:15px;letter-spacing:.16em;fill:var(--accent)}
.diagram .stair{fill:none;stroke:var(--accent);stroke-width:1.5}
.column .diagram,.split-panel .diagram{max-height:100%}
.r-stagger .slide.active .diagram .node,.r-stagger .slide.active .diagram .hub,.r-stagger .slide.active .diagram .point{animation:rv .9s var(--ease) both;animation-delay:calc(var(--i,0) * 60ms + 400ms)}
.r-stagger .slide.active .diagram .edge,.r-stagger .slide.active .diagram .arc,.r-stagger .slide.active .diagram .stair,.r-stagger .slide.active .diagram .shape{stroke-dasharray:2400;stroke-dashoffset:2400;animation:draw 2s var(--ease) .5s forwards}
.block-table{width:100%;border-collapse:collapse;font-size:21px;line-height:1.4;font-weight:300}
.block-table th{text-align:left;font-family:var(--font-mono);font-size:13px;letter-spacing:.2em;text-transform:uppercase;color:var(--muted);font-weight:400;padding:0 28px 16px 0;border-bottom:1px solid var(--hair-strong)}
.block-table td{vertical-align:top;padding:22px 28px 22px 0;border-bottom:1px solid var(--hair);color:var(--muted)}
.block-table td:first-child{font-family:var(--font-heading);font-size:32px;line-height:1.1;color:var(--ink);letter-spacing:-0.015em;white-space:nowrap;padding-right:40px}
.block-table td:last-child,.block-table th:last-child{padding-right:0}
.block-table.dense{font-size:19px}
.block-table.dense td{padding:16px 24px 16px 0}
.block-table.dense td:first-child{font-size:28px}
.chart-title{font-family:var(--font-mono);font-size:13px;letter-spacing:.2em;text-transform:uppercase;color:var(--muted)}
.chart{width:100%;height:100%;min-height:0;flex:1;overflow:visible;--chart-0:var(--accent);--chart-1:var(--ink);--chart-2:var(--accent-2);--chart-3:var(--muted);--chart-4:${mix(c.accent, c.background, 0.5)};--chart-5:${mix(c.ink, c.background, 0.6)}}
.chart text{font-family:var(--font-mono);fill:var(--ink)}
.chart-value{font-size:22px;font-weight:500}
.chart-label{font-size:17px;fill:var(--muted);letter-spacing:.04em}
.chart-legend{font-size:20px}
.chart-pct{fill:var(--muted);font-size:18px}
.chart-total{font-size:64px;font-family:var(--font-heading)}
.chart-axis{stroke:var(--hair-strong);stroke-width:1}
.chart-line{fill:none;stroke:var(--accent);stroke-width:3;stroke-linecap:round;stroke-linejoin:round}
.chart .dot circle{fill:var(--bg);stroke:var(--accent);stroke-width:2}
.chart .bar rect{transform-origin:center bottom;transform-box:fill-box;rx:2}
.r-stagger .slide.active .chart .bar rect{animation:grow .9s var(--ease) both;animation-delay:calc(var(--i,0) * 70ms + 300ms)}
.r-stagger .slide.active .chart .arc{animation:sweep 1.1s var(--ease) both;animation-delay:calc(var(--i,0) * 90ms + 300ms)}
.r-stagger .slide.active .chart-line{stroke-dasharray:3000;stroke-dashoffset:3000;animation:draw 1.6s var(--ease) .3s forwards}
@keyframes grow{from{transform:scaleY(0)}to{transform:scaleY(1)}}
@keyframes sweep{from{stroke-dasharray:0 2000}}
@keyframes draw{to{stroke-dashoffset:0}}
.motif{position:absolute;right:-60px;top:50%;transform:translateY(-50%);width:820px;height:820px;color:var(--accent);opacity:${dark ? 0.55 : 0.4};z-index:-1;pointer-events:none}
.motif .fill{fill:currentColor;stroke:none}
.motif .faint{opacity:.35}
.motif .sweep{fill:currentColor;opacity:.08;stroke:none}
.motif .spin{transform-origin:500px 500px;animation:spin 120s linear infinite}
@keyframes spin{to{transform:rotate(360deg)}}
.layout-title .motif{right:-120px;top:40%;width:980px;height:980px}
.layout-statement .motif{right:auto;left:50%;top:50%;transform:translate(-50%,-50%);width:1300px;height:1300px;opacity:${dark ? 0.16 : 0.12}}
.layout-section .motif{right:auto;left:900px;top:auto;bottom:-260px;transform:none;width:760px;height:760px;opacity:.3}
.layout-split .motif{right:-300px;width:900px;height:900px;opacity:.12}
.variant-accent .motif{color:var(--accent-ink);opacity:.25}
.autofit{zoom:var(--fit,1)}
.notes{display:none;position:absolute;left:0;right:0;bottom:0;padding:32px 96px;background:rgba(0,0,0,.88);color:#fff;font-size:24px;line-height:1.45;z-index:3;font-weight:300}
body.show-notes .notes{display:block}
.progress{position:fixed;left:0;bottom:0;height:2px;background:var(--accent);transition:width .6s var(--ease);z-index:5}
.nav{position:fixed;right:28px;bottom:22px;display:flex;gap:8px;z-index:5;opacity:0;transition:opacity .4s var(--ease)}
body.show-nav .nav{opacity:1}
.nav button{width:44px;height:44px;border-radius:50%;border:1px solid rgba(255,255,255,.2);background:rgba(0,0,0,.5);color:#fff;font-size:18px;cursor:pointer}
.nav button:hover{background:rgba(255,255,255,.12)}
.rv{opacity:0;transform:translateY(18px);clip-path:inset(0 0 100% -20px)}
.editing [data-block-id]{cursor:pointer;outline:1px solid transparent;outline-offset:10px;border-radius:2px;transition:outline-color .2s}
.editing [data-block-id]:hover{outline-color:var(--hair-strong)}
.editing [data-block-id].selected{outline-color:var(--accent)}
.editing .nav,.editing .progress{display:none}
.editing .stars{animation:none}
.r-none .rv{opacity:1;transform:none;clip-path:none}
.r-stagger .slide.active .rv{animation:rv 1.1s var(--ease) forwards;animation-delay:calc(var(--i,0) * 90ms + 160ms)}
.r-step .slide.active .rv.on{animation:rv .8s var(--ease) forwards}
@keyframes rv{60%{clip-path:inset(0 0 0 -20px)}to{opacity:1;transform:none;clip-path:inset(-20px -20px -20px -20px)}}
body.overview .frame{position:static;transform:none!important;width:100vw;height:100vh;display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:24px;padding:32px;overflow:auto;align-content:start;box-sizing:border-box}
body.overview .slide{position:relative;inset:auto;opacity:1!important;visibility:visible!important;transform:none!important;transition:none!important;zoom:.22;cursor:pointer;outline:4px solid transparent;pointer-events:auto}
body.overview .slide.active{outline-color:var(--accent)}
body.overview .slide .rv{opacity:1!important;transform:none!important;clip-path:none!important;animation:none!important}
body.overview .progress,body.overview .nav{display:none}
strong{font-weight:500}
code{font-family:var(--font-mono);background:var(--accent-soft);padding:.05em .3em;border-radius:3px;font-size:.9em}
@media (prefers-reduced-motion:reduce){.slide,.stars,.progress{animation:none!important;transition:none!important}.rv{opacity:1!important;transform:none!important;clip-path:none!important;animation:none!important}}
@media print{html,body{background:#fff;overflow:visible;height:auto}.stage{position:static;display:block;overflow:visible}.frame{position:static;transform:none!important;width:auto;height:auto}.slide{position:relative;display:flex!important;opacity:1!important;visibility:visible!important;transform:none!important;transition:none!important;page-break-after:always;break-after:page}.rv{opacity:1!important;transform:none!important;clip-path:none!important}.stars{animation:none!important}.notes,.progress,.nav{display:none!important}@page{size:${SLIDE_WIDTH}px ${SLIDE_HEIGHT}px;margin:0}}
`
}

const DECK_SCRIPT = `
(function(){
  var slides = Array.prototype.slice.call(document.querySelectorAll('.slide'));
  var frame = document.querySelector('.frame');
  var progress = document.querySelector('.progress');
  var body = document.body;
  var total = slides.length;
  var current = -1;
  var step = body.classList.contains('r-step');
  var navTimer = null;
  function fromHash(){ var n = parseInt(location.hash.replace('#','').replace('slide-',''), 10); return isNaN(n) ? 0 : Math.min(total - 1, Math.max(0, n - 1)); }
  function pending(slide){ return Array.prototype.slice.call(slide.querySelectorAll('.rv')).filter(function(el){ return !el.classList.contains('on'); }); }
  function shown(slide){ return Array.prototype.slice.call(slide.querySelectorAll('.rv.on')); }
  function show(i, direction){
    var next = Math.min(total - 1, Math.max(0, i));
    if (next === current) return;
    body.classList.toggle('backward', direction < 0);
    if (current >= 0) {
      var prev = slides[current];
      prev.classList.remove('active');
      prev.classList.add('leaving');
      setTimeout(function(){ prev.classList.remove('leaving'); }, 900);
    }
    current = next;
    var slide = slides[current];
    if (step) {
      var items = slide.querySelectorAll('.rv');
      for (var k = 0; k < items.length; k++) items[k].classList.toggle('on', direction < 0);
    }
    slide.classList.add('active');
    autofit(slide);
    broadcast();
    if (progress) progress.style.width = ((current + 1) / total * 100) + '%';
    try { if (!EDITING && location.hash !== '#' + (current + 1)) history.replaceState(null, '', '#' + (current + 1)); } catch (err) {}
    if (window.parent !== window) window.parent.postMessage({ type: 'slides:navigate', index: current, total: total }, '*');
  }
  function forward(){
    if (step) { var p = pending(slides[current]); if (p.length) { p[0].classList.add('on'); return; } }
    show(current + 1, 1);
  }
  function backward(){
    if (step) { var s = shown(slides[current]); if (s.length) { s[s.length - 1].classList.remove('on'); return; } }
    show(current - 1, -1);
  }
  function autofit(slide){
    var root = slide.querySelector(':scope > .body > .stack, :scope > .body > .split, :scope > .stack, :scope > .split');
    if (!root) return;
    root.classList.add('autofit');
    var levels = [1, .94, .88, .82, .76, .7];
    for (var i = 0; i < levels.length; i++) {
      root.style.setProperty('--fit', levels[i]);
      if (root.scrollHeight <= root.clientHeight + 2) break;
    }
  }
  var channel = null;
  try { channel = new BroadcastChannel('slides:' + DECK_ID); } catch (err) { channel = null; }
  function broadcast(){ if (channel) channel.postMessage({ type: 'state', index: current, total: total, notes: NOTES[current] || '', title: TITLES[current] || '', next: TITLES[current + 1] || '' }); }
  function openSpeaker(){
    var win = window.open('', 'slides-speaker-' + DECK_ID, 'width=960,height=640');
    if (!win) return;
    win.document.write(SPEAKER_HTML);
    win.document.close();
    setTimeout(broadcast, 300);
  }
  if (channel) channel.onmessage = function(e){ var d = e.data || {}; if (d.type === 'hello') broadcast(); if (d.type === 'goto') show(d.index, d.index > current ? 1 : -1); if (d.type === 'forward') forward(); if (d.type === 'backward') backward(); };
  function fit(){
    var w = window.innerWidth, h = window.innerHeight;
    var scale = Math.min(w / ${SLIDE_WIDTH}, h / ${SLIDE_HEIGHT});
    if (frame) frame.style.transform = 'translate(-50%,-50%) scale(' + scale + ')';
  }
  function wakeNav(){ body.classList.add('show-nav'); clearTimeout(navTimer); navTimer = setTimeout(function(){ body.classList.remove('show-nav'); }, 2200); }
  document.addEventListener('keydown', function(e){
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown' || e.key === ' ' || e.key === 'PageDown' || e.key === 'Enter') { e.preventDefault(); forward(); }
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp' || e.key === 'PageUp' || e.key === 'Backspace') { e.preventDefault(); backward(); }
    else if (e.key === 'Home') show(0, -1);
    else if (e.key === 'End') show(total - 1, 1);
    else if (e.key === 'n' || e.key === 'N') body.classList.toggle('show-notes');
    else if (e.key === 'o' || e.key === 'O' || (e.key === 'Escape' && body.classList.contains('overview'))) { body.classList.toggle('overview'); if (!body.classList.contains('overview')) fit(); }
    else if (e.key === 's' || e.key === 'S') openSpeaker();
    else if (e.key === 'f' || e.key === 'F') { if (document.fullscreenElement) document.exitFullscreen(); else document.documentElement.requestFullscreen && document.documentElement.requestFullscreen(); }
  });
  document.addEventListener('mousemove', wakeNav);
  var startX = null;
  document.addEventListener('touchstart', function(e){ startX = e.touches[0].clientX; }, { passive: true });
  document.addEventListener('touchend', function(e){ if (startX === null) return; var dx = e.changedTouches[0].clientX - startX; if (Math.abs(dx) > 40) { dx < 0 ? forward() : backward(); } startX = null; });
  var prevBtn = document.querySelector('.nav .prev'); var nextBtn = document.querySelector('.nav .next');
  if (prevBtn) prevBtn.addEventListener('click', backward);
  if (nextBtn) nextBtn.addEventListener('click', forward);
  var stage = document.querySelector('.stage');
  if (stage) stage.addEventListener('click', function(e){ if (e.target.closest && e.target.closest('.nav')) return; if (EDITING) { var block = e.target.closest && e.target.closest('[data-block-id]'); var slideEl = e.target.closest && e.target.closest('.slide'); if (window.parent !== window) window.parent.postMessage({ type: 'slides:select', blockId: block ? block.getAttribute('data-block-id') : null, index: slideEl ? parseInt(slideEl.getAttribute('data-index'), 10) : current }, '*'); return; } if (body.classList.contains('overview')) { var target = e.target.closest && e.target.closest('.slide'); if (target) { body.classList.remove('overview'); fit(); show(parseInt(target.getAttribute('data-index'), 10), 1); } return; } var x = e.clientX / window.innerWidth; x < 0.2 ? backward() : forward(); });
  window.addEventListener('message', function(e){ var d = e.data || {}; if (d.type !== 'slides:highlight') return; var all = document.querySelectorAll('[data-block-id].selected'); for (var k = 0; k < all.length; k++) all[k].classList.remove('selected'); if (d.blockId) { var el = document.querySelector('[data-block-id="' + d.blockId + '"]'); if (el) el.classList.add('selected'); } });
  if (EDITING) { document.addEventListener('keydown', function(e){ e.stopImmediatePropagation(); }, true); }
  window.addEventListener('hashchange', function(){ show(fromHash(), 1); });
  window.addEventListener('resize', fit);
  window.addEventListener('message', function(e){ var d = e.data || {}; if (d.type === 'slides:goto' && typeof d.index === 'number') show(d.index, d.index > current ? 1 : -1); });
  fit();
  show(fromHash(), 1);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(function(){ slides.forEach(autofit); });
  window.addEventListener('load', function(){ slides.forEach(autofit); });
})();
`

export function scriptJson(value: unknown): string {
  return JSON.stringify(value).replace(/</g, '\\u003c').replace(/>/g, '\\u003e').replace(/&/g, '\\u0026')
}

function speakerHtml(deck: Deck, theme: ResolvedTheme): string {
  return `<!doctype html><html><head><meta charset="utf-8"><title>Speaker: ${escapeHtml(deck.title)}</title><style>
body{margin:0;background:#0b0b10;color:#f4f4f8;font-family:'${theme.fonts.body}',ui-sans-serif,system-ui,sans-serif;display:grid;grid-template-rows:auto 1fr auto;height:100vh}
header{display:flex;justify-content:space-between;align-items:center;padding:16px 24px;border-bottom:1px solid #222;font-size:14px;letter-spacing:.1em;text-transform:uppercase;color:#9aa}
#timer{font-family:ui-monospace,Menlo,monospace;font-size:28px;color:#fff;letter-spacing:0}
main{display:grid;grid-template-columns:1.4fr 1fr;gap:24px;padding:24px;min-height:0}
.card{background:#15151d;border:1px solid #262633;border-radius:16px;padding:24px;min-height:0;overflow:auto}
.label{font-size:12px;letter-spacing:.14em;text-transform:uppercase;color:#8a8aa0;margin-bottom:12px}
#notes{font-size:26px;line-height:1.5}
#title{font-size:20px;line-height:1.3;margin-bottom:24px;color:#fff}
#next{font-size:20px;line-height:1.3;color:#cfd}
footer{display:flex;gap:12px;justify-content:center;padding:14px;border-top:1px solid #222}
button{background:#26263a;color:#fff;border:0;border-radius:10px;padding:10px 18px;font-size:15px;cursor:pointer}
button:hover{background:#34344d}
</style></head><body>
<header><span>${escapeHtml(deck.title)}</span><span id="pos"></span><span id="timer">00:00</span></header>
<main><section class="card"><div class="label">Current slide</div><div id="title"></div><div class="label">Speaker notes</div><div id="notes"></div></section><section class="card"><div class="label">Up next</div><div id="next"></div></section></main>
<footer><button id="prev">&larr; Previous</button><button id="reset">Reset timer</button><button id="nextBtn">Next &rarr;</button></footer>
<script>
var ch = new BroadcastChannel('slides:' + ${scriptJson(deck.id)});
var start = Date.now();
function tick(){ var s = Math.floor((Date.now() - start) / 1000); document.getElementById('timer').textContent = String(Math.floor(s / 60)).padStart(2, '0') + ':' + String(s % 60).padStart(2, '0'); }
setInterval(tick, 1000);
ch.onmessage = function(e){ var d = e.data || {}; if (d.type !== 'state') return; document.getElementById('pos').textContent = (d.index + 1) + ' / ' + d.total; document.getElementById('title').textContent = d.title; document.getElementById('notes').textContent = d.notes || 'No notes for this slide.'; document.getElementById('next').textContent = d.next || 'End of deck'; };
document.getElementById('prev').onclick = function(){ ch.postMessage({ type: 'backward' }); };
document.getElementById('nextBtn').onclick = function(){ ch.postMessage({ type: 'forward' }); };
document.getElementById('reset').onclick = function(){ start = Date.now(); tick(); };
document.addEventListener('keydown', function(e){ if (e.key === 'ArrowRight' || e.key === ' ') ch.postMessage({ type: 'forward' }); if (e.key === 'ArrowLeft') ch.postMessage({ type: 'backward' }); });
ch.postMessage({ type: 'hello' });
</script></body></html>`
}

export interface RenderOptions {
  editing?: boolean
}

export function renderDeckHtml(deck: Deck, brand?: ThemeOverrides, options: RenderOptions = {}): string {
  const theme = resolveTheme(deck, brand)
  const total = deck.slides.length
  const transition = options.editing ? 'none' : (deck.motion?.transition ?? 'fade')
  const reveal = options.editing ? 'none' : (deck.motion?.reveal ?? 'stagger')
  const slides = deck.slides.map((slide, index) => renderSlide(slide, index, total, theme, deck)).join('\n')
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${escapeHtml(deck.title)}</title>
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link rel="stylesheet" href="${googleFontsHref(theme)}" />
<style>${deckCss(theme)}</style>
</head>
<body class="deck t-${transition} r-${reveal}${options.editing ? ' editing' : ''}">
<main class="stage"><div class="frame">
${slides}
</div></main>
<div class="progress"></div>
<div class="nav" aria-label="Navigation"><button type="button" class="prev" aria-label="Previous slide">&larr;</button><button type="button" class="next" aria-label="Next slide">&rarr;</button></div>
<script>var DECK_ID=${scriptJson(deck.id)};var EDITING=${options.editing ? 'true' : 'false'};var NOTES=${scriptJson(deck.slides.map((slide) => slide.notes ?? ''))};var TITLES=${scriptJson(deck.slides.map((slide) => slide.title ?? ''))};var SPEAKER_HTML=${scriptJson(speakerHtml(deck, theme))};</script>
<script>${DECK_SCRIPT}</script>
</body>
</html>
`
}
