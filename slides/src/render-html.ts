import { iconSvg } from './deck-icons.js'
import type { Block, Deck, Entry, Slide, ThemeOverrides } from './model.js'
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
          `<div${rv.wrap('card glass')}>${entry.icon ? `<div class="card-icon">${iconSvg(entry.icon)}</div>` : block.numbered ? `<div class="card-number">${String(index + 1).padStart(2, '0')}</div>` : ''}<div class="card-title">${inline(entry.title)}</div>${entry.text ? `<div class="card-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    }
    case 'steps':
      return `<div class="block block-steps${block.entries.length > 4 ? ' dense' : ''}">${entries(
        block.entries,
        (entry, index) =>
          `<div${rv.wrap('step glass')}><div class="step-number">${entry.icon ? iconSvg(entry.icon) : index + 1}</div><div class="step-title">${inline(entry.title)}</div>${entry.text ? `<div class="step-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    case 'timeline':
      return `<div class="block block-timeline" style="--cols:${Math.max(1, block.entries.length)}">${entries(
        block.entries,
        (entry) =>
          `<div${rv.wrap('milestone')}><div class="milestone-dot"></div><div class="milestone-title">${inline(entry.title)}</div>${entry.text ? `<div class="milestone-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    case 'chart':
      return `<figure${rv.wrap('block block-chart glass')}>${block.title ? `<figcaption class="chart-title">${inline(block.title)}</figcaption>` : ''}${chartSvg(block.kind, block.series, block.unit, block.title)}</figure>`
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
  const kicker = slide.kicker ? `<div${rv.wrap('kicker')}>${inline(slide.kicker)}</div>` : ''
  const title = slide.title ? `<h2${rv.wrap('slide-title')}>${inline(slide.title)}</h2>` : ''
  const subtitle = slide.subtitle ? `<p${rv.wrap('slide-subtitle')}>${inline(slide.subtitle)}</p>` : ''
  const blocks = () => slide.blocks.map((block) => renderBlock(block, rv)).join('')
  const columns = () => {
    const [left, right] = splitColumns(slide.blocks)
    return `<div class="columns"><div class="column">${left.map((block) => renderBlock(block, rv)).join('')}</div><div class="column">${right.map((block) => renderBlock(block, rv)).join('')}</div></div>`
  }
  switch (slide.layout) {
    case 'title':
      return `<div class="stack center hero">${kicker}${slide.title ? `<h1${rv.wrap('deck-title')}>${inline(slide.title)}</h1>` : ''}${subtitle}<div${rv.wrap('hero-meta')}><div class="accent-bar"></div>${deck.author ? `<span>${inline(deck.author)}</span>` : ''}</div><div class="blocks">${blocks()}</div></div>`
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
  deck: Pick<Deck, 'author'>,
): string {
  const background = slide.background
    ? /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(slide.background)
      ? ` style="--bg:${escapeHtml(slide.background)}"`
      : safeUrl(slide.background)
        ? ` style="background-image:url('${safeUrl(slide.background)}')"`
        : ''
    : ''
  const footer = `<footer class="slide-footer"><span>${theme.footer ? inline(theme.footer) : ''}</span><span class="slide-number">${String(index + 1).padStart(2, '0')} <em>/ ${String(total).padStart(2, '0')}</em></span></footer>`
  const notes = slide.notes ? `<aside class="notes">${inline(slide.notes)}</aside>` : ''
  const variant = slide.variant ?? 'default'
  return `<section class="slide layout-${slide.layout} variant-${variant}" data-index="${index}" id="slide-${index + 1}"${background}><div class="mesh" aria-hidden="true"><i class="blob blob-a"></i><i class="blob blob-b"></i><i class="blob blob-c"></i></div>${motifSvg(slide.visual)}${renderSlideBody(slide, index, deck)}${footer}${notes}</section>`
}

export function deckCss(theme: ResolvedTheme): string {
  const c = theme.colors
  const dark = theme.dark
  const accent2 = hueShift(c.accent, dark ? 48 : -32)
  const gradientEnd = mix(c.background, c.accent, dark ? 0.38 : 0.2)
  const glassA = dark ? 'rgba(255,255,255,0.09)' : 'rgba(255,255,255,0.72)'
  const glassB = dark ? 'rgba(255,255,255,0.03)' : 'rgba(255,255,255,0.42)'
  const glassBorder = dark ? 'rgba(255,255,255,0.14)' : withAlpha(c.ink, 0.1)
  const glassHi = dark ? 'rgba(255,255,255,0.18)' : 'rgba(255,255,255,0.9)'
  return `
:root{--bg:${c.background};--surface:${c.surface};--ink:${c.ink};--muted:${c.muted};--accent:${c.accent};--accent-2:${accent2};--accent-ink:${c.accent_ink};--accent-soft:${withAlpha(c.accent, 0.16)};--accent-glow:${withAlpha(c.accent, dark ? 0.42 : 0.26)};--accent-2-glow:${withAlpha(accent2, dark ? 0.34 : 0.2)};--gradient-end:${gradientEnd};--grid:${withAlpha(c.ink, dark ? 0.04 : 0.055)};--glass-a:${glassA};--glass-b:${glassB};--glass-border:${glassBorder};--glass-hi:${glassHi};--shadow:${dark ? '0 30px 80px rgba(0,0,0,.45)' : '0 30px 80px rgba(20,20,40,.14)'};--font-heading:'${theme.fonts.heading}',ui-sans-serif,system-ui,sans-serif;--font-body:'${theme.fonts.body}',ui-sans-serif,system-ui,sans-serif;--font-mono:'${theme.fonts.mono}',ui-monospace,SFMono-Regular,Menlo,monospace;--radius:${theme.radius}px;--heading-weight:${theme.heading_weight};--w:${SLIDE_WIDTH}px;--h:${SLIDE_HEIGHT}px;--ease:cubic-bezier(.2,.7,.2,1)}
*{box-sizing:border-box}
html,body{margin:0;background:#000;color:var(--ink);font-family:var(--font-body);height:100%;-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility}
body.deck{overflow:hidden}
.stage{position:fixed;inset:0;display:grid;place-items:center;perspective:2000px}
.frame{position:relative;width:var(--w);height:var(--h);transform-origin:center center}
.slide{position:absolute;inset:0;width:var(--w);height:var(--h);background:var(--bg);color:var(--ink);padding:88px 120px 72px;overflow:hidden;background-size:cover;background-position:center;display:flex;flex-direction:column;isolation:isolate;opacity:0;visibility:hidden;pointer-events:none;will-change:opacity,transform}
.slide.active{opacity:1;visibility:visible;pointer-events:auto;z-index:2}
.slide.leaving{visibility:visible;z-index:1}
.t-fade .slide{transition:opacity .7s var(--ease),transform .9s var(--ease);transform:scale(1.035)}
.t-fade .slide.active{transform:scale(1)}
.t-fade .slide.leaving{opacity:0;transform:scale(.985)}
.t-slide .slide{transition:opacity .6s var(--ease),transform .8s var(--ease);transform:translateX(8%)}
.t-slide .slide.active{transform:translateX(0)}
.t-slide .slide.leaving{opacity:0;transform:translateX(-8%)}
.t-slide.backward .slide{transform:translateX(-8%)}
.t-slide.backward .slide.leaving{transform:translateX(8%)}
.t-zoom .slide{transition:opacity .6s var(--ease),transform .9s var(--ease);transform:scale(.86)}
.t-zoom .slide.active{transform:scale(1)}
.t-zoom .slide.leaving{opacity:0;transform:scale(1.12)}
.t-none .slide{transition:none}
.mesh{position:absolute;inset:-20%;z-index:-2;pointer-events:none;filter:blur(70px) saturate(1.15);opacity:${dark ? 0.9 : 0.75}}
.blob{position:absolute;border-radius:50%;display:block}
.blob-a{width:62%;height:62%;right:-8%;top:-14%;background:radial-gradient(circle at 40% 40%,var(--accent-glow),transparent 62%);animation:drift-a 26s var(--ease) infinite alternate}
.blob-b{width:48%;height:48%;left:-6%;bottom:-16%;background:radial-gradient(circle at 60% 60%,var(--accent-2-glow),transparent 64%);animation:drift-b 31s var(--ease) infinite alternate}
.blob-c{width:34%;height:34%;left:38%;top:52%;background:radial-gradient(circle,var(--accent-soft),transparent 66%);animation:drift-c 23s var(--ease) infinite alternate}
@keyframes drift-a{to{transform:translate(-9%,12%) scale(1.12)}}
@keyframes drift-b{to{transform:translate(12%,-10%) scale(1.08)}}
@keyframes drift-c{to{transform:translate(-14%,-16%) scale(1.2)}}
.slide::after{content:'';position:absolute;inset:0;background-image:linear-gradient(var(--grid) 1px,transparent 1px),linear-gradient(90deg,var(--grid) 1px,transparent 1px);background-size:80px 80px;mask-image:radial-gradient(ellipse at 72% 28%,#000 0%,transparent 72%);pointer-events:none;z-index:-1}
.layout-title .blob-a{width:92%;height:92%;right:-20%;top:-30%}
.layout-title .blob-b{width:66%;height:66%}
.variant-accent{--bg:var(--accent);--ink:var(--accent-ink);--muted:${withAlpha(c.accent_ink, 0.74)};--surface:${withAlpha(c.accent_ink, 0.12)};--accent-soft:${withAlpha(c.accent_ink, 0.2)};--grid:${withAlpha(c.accent_ink, 0.09)};--glass-a:${withAlpha(c.accent_ink, 0.16)};--glass-b:${withAlpha(c.accent_ink, 0.06)};--glass-border:${withAlpha(c.accent_ink, 0.25)};--glass-hi:${withAlpha(c.accent_ink, 0.35)}}
.variant-accent .blob-a{background:radial-gradient(circle,rgba(255,255,255,.38),transparent 62%)}
.variant-accent .blob-b{background:radial-gradient(circle,rgba(0,0,0,.34),transparent 64%)}
.variant-accent .blob-c{background:radial-gradient(circle,${withAlpha(accent2, 0.5)},transparent 66%)}
.variant-accent .kicker,.variant-accent .metric-value,.variant-accent .step-number,.variant-accent .card-number,.variant-accent .section-decor{color:var(--accent-ink)}
.variant-accent .kicker::before,.variant-accent .accent-bar,.variant-accent .block-bullets li::before,.variant-accent .block-quote,.variant-accent .milestone-dot,.variant-accent .block-timeline::before{background:var(--accent-ink);border-color:var(--accent-ink)}
.variant-accent .section-decor{-webkit-text-stroke-color:var(--accent-ink)}
.variant-accent .step-number{background:${withAlpha(c.accent_ink, 0.2)}}
.variant-accent .deck-title,.variant-accent .statement .slide-title{background:none;-webkit-text-fill-color:currentColor;color:var(--accent-ink)}
.variant-gradient{background:linear-gradient(135deg,var(--bg) 0%,var(--gradient-end) 100%)}
.variant-muted{--bg:var(--surface)}
.glass{background:linear-gradient(135deg,var(--glass-a),var(--glass-b));border:1px solid var(--glass-border);box-shadow:var(--shadow),inset 0 1px 0 var(--glass-hi);backdrop-filter:blur(24px) saturate(1.2);-webkit-backdrop-filter:blur(24px) saturate(1.2);border-radius:var(--radius)}
.stack{display:flex;flex-direction:column;gap:28px;flex:1;min-height:0}
.head{display:flex;flex-direction:column;gap:14px;padding-bottom:12px}
.stack.center{justify-content:center;align-items:flex-start;max-width:1240px}
.stack.statement{align-items:center;text-align:center;max-width:none;justify-content:center}
.stack.statement .slide-title{font-size:92px;line-height:1.02;max-width:1320px;letter-spacing:-0.03em;background:linear-gradient(120deg,var(--ink) 0%,var(--ink) 60%,var(--accent) 100%);-webkit-background-clip:text;background-clip:text;-webkit-text-fill-color:transparent}
.stack.statement .slide-subtitle{text-align:center;max-width:1000px}
.stack.statement .kicker{justify-content:center}
.kicker{display:inline-flex;align-items:center;gap:14px;font-size:19px;font-weight:600;letter-spacing:.22em;text-transform:uppercase;color:var(--accent)}
.kicker::before{content:'';width:28px;height:2px;background:var(--accent);border-radius:2px}
.hero .kicker{margin-bottom:6px}
.hero-meta{display:flex;align-items:center;gap:24px;color:var(--muted);font-size:22px;margin-top:10px;letter-spacing:.02em}
.accent-bar{width:120px;height:8px;border-radius:999px;background:linear-gradient(90deg,var(--accent),var(--accent-2))}
.deck-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:116px;line-height:.98;letter-spacing:-0.035em;margin:0;max-width:1300px;text-wrap:balance;background:linear-gradient(120deg,var(--ink) 0%,var(--ink) 58%,var(--accent) 100%);-webkit-background-clip:text;background-clip:text;-webkit-text-fill-color:transparent;padding-bottom:.08em;margin-bottom:-.08em}
.slide-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:62px;line-height:1.06;letter-spacing:-0.025em;margin:0;text-wrap:balance}
.slide-subtitle{font-size:31px;line-height:1.4;color:var(--muted);margin:0;max-width:1180px;font-weight:400}
.section{justify-content:center;max-width:1100px}
.section .slide-title{font-size:92px;letter-spacing:-0.035em}
.section-decor{position:absolute;right:60px;bottom:-40px;font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:560px;line-height:1;letter-spacing:-0.06em;color:transparent;-webkit-text-stroke:2px var(--accent);opacity:.32;z-index:-1;pointer-events:none;user-select:none}
.blocks{display:flex;flex-direction:column;gap:26px;min-height:0;flex:1}
.columns{display:grid;grid-template-columns:1fr 1fr;gap:56px;flex:1;min-height:0}
.column{display:flex;flex-direction:column;gap:22px;min-width:0}
.split{display:grid;grid-template-columns:5fr 6fr;gap:72px;flex:1;min-height:0;align-items:stretch}
.split-copy{display:flex;flex-direction:column;gap:22px;justify-content:center;min-width:0}
.split-copy .slide-title{font-size:66px}
.split-panel{padding:44px;display:flex;flex-direction:column;min-height:0;position:relative;overflow:hidden}
.split-panel::before{content:'';position:absolute;inset:auto -30% -40% auto;width:70%;height:70%;background:radial-gradient(circle,var(--accent-glow),transparent 65%);filter:blur(40px);pointer-events:none}
.split-panel .blocks{gap:22px;position:relative}
.split-panel .block-metric,.split-panel .card,.split-panel .step{background:${dark ? 'rgba(255,255,255,0.06)' : 'rgba(255,255,255,0.65)'};box-shadow:none;backdrop-filter:none;-webkit-backdrop-filter:none}
.split-panel .block-cards{--cols:2}
.block{margin:0}
.block-heading{font-family:var(--font-heading);font-size:36px;font-weight:var(--heading-weight);line-height:1.2;letter-spacing:-0.015em}
.block-text{font-size:29px;line-height:1.5;max-width:1240px;color:${dark ? withAlpha(c.ink, 0.92) : c.ink}}
.block-bullets{font-size:29px;line-height:1.42;padding-left:0;list-style:none;display:flex;flex-direction:column;gap:16px}
.block-bullets.dense{font-size:25px;gap:10px}
.block-bullets li{position:relative;padding-left:44px}
.block-bullets li::before{content:'';position:absolute;left:0;top:.52em;width:12px;height:12px;border-radius:3px;background:linear-gradient(135deg,var(--accent),var(--accent-2));transform:rotate(45deg)}
.block-image{display:flex;flex-direction:column;gap:12px;align-items:flex-start;min-height:0;flex:1}
.block-image img{max-width:100%;max-height:100%;object-fit:contain;border-radius:var(--radius);box-shadow:var(--shadow)}
.block-image figcaption{font-size:21px;color:var(--muted)}
.image-missing{padding:48px;border:2px dashed var(--muted);border-radius:var(--radius);color:var(--muted);font-size:24px}
.block-code{padding:30px 36px;font-family:var(--font-mono);font-size:22px;line-height:1.55;overflow:hidden;position:relative;white-space:pre-wrap;word-break:break-word}
.block-code[data-language]::before{content:attr(data-language);position:absolute;top:14px;right:20px;font-size:14px;color:var(--muted);text-transform:uppercase;letter-spacing:.1em}
.block-quote{position:relative;padding:18px 0 12px 48px;display:flex;flex-direction:column;gap:18px;border-left:3px solid var(--accent)}
.block-quote p{margin:0;font-family:var(--font-heading);font-size:44px;line-height:1.28;font-style:italic;text-wrap:balance;letter-spacing:-0.01em}
.block-quote cite{font-style:normal;font-size:22px;color:var(--muted);letter-spacing:.04em}
.statement .block-quote{border-left:0;padding:60px 0 0;align-items:center}
.statement .block-quote::before{content:'\\201C';position:absolute;top:-70px;left:50%;transform:translateX(-50%);font-family:var(--font-heading);font-size:260px;line-height:1;color:var(--accent);opacity:.35}
.statement .block-quote p{font-size:56px}
.block-metric{display:flex;flex-direction:column;gap:6px;padding:30px 36px;align-self:flex-start;min-width:340px}
.metric-value{font-family:var(--font-heading);font-size:104px;line-height:1;font-weight:var(--heading-weight);letter-spacing:-0.04em;background:linear-gradient(120deg,var(--accent),var(--accent-2));-webkit-background-clip:text;background-clip:text;-webkit-text-fill-color:transparent}
.metric-label{font-size:23px;color:var(--muted);letter-spacing:.01em}
.column .block-metric{align-self:stretch}
.block-cards{display:grid;grid-template-columns:repeat(var(--cols,3),minmax(0,1fr));gap:20px;flex:1;min-height:0;align-content:start}
.card{display:flex;flex-direction:column;gap:10px;padding:28px 30px;min-width:0;position:relative}
.card-number{font-family:var(--font-mono);font-size:16px;letter-spacing:.14em;color:var(--accent);font-weight:600}
.card-title{font-family:var(--font-heading);font-size:29px;font-weight:var(--heading-weight);line-height:1.18;text-wrap:balance;letter-spacing:-0.015em}
.card-text{font-size:21px;line-height:1.42;color:var(--muted)}
.block-cards.dense .card{padding:20px 22px;gap:6px}
.block-cards.dense .card-title{font-size:24px}
.block-cards.dense .card-text{font-size:17.5px}
.block-steps{display:flex;gap:22px;align-items:stretch;flex:1;min-height:0;flex-wrap:wrap}
.step{flex:1 1 0;min-width:200px;display:flex;flex-direction:column;gap:12px;padding:30px 28px;position:relative}
.step:not(:last-child)::after{content:'';position:absolute;right:-22px;top:50%;width:22px;height:2px;background:linear-gradient(90deg,var(--accent),var(--accent-2));z-index:1}
.step-number{width:46px;height:46px;border-radius:50%;background:var(--accent-soft);color:var(--accent);display:grid;place-items:center;font-family:var(--font-mono);font-size:20px;font-weight:700}
.step-title{font-family:var(--font-heading);font-size:29px;font-weight:var(--heading-weight);line-height:1.15;text-wrap:balance;letter-spacing:-0.015em}
.step-text{font-size:20px;line-height:1.42;color:var(--muted)}
.block-steps.dense .step{padding:22px 20px;gap:8px}
.block-steps.dense .step-title{font-size:24px}
.block-steps.dense .step-text{font-size:17.5px}
.block-timeline{display:grid;grid-template-columns:repeat(var(--cols,4),minmax(0,1fr));gap:24px;position:relative;padding-top:40px;margin-top:12px}
.block-timeline::before{content:'';position:absolute;left:12px;right:12px;top:12px;height:2px;background:linear-gradient(90deg,var(--accent),var(--accent-2));opacity:.6;border-radius:2px}
.milestone{display:flex;flex-direction:column;gap:10px;position:relative;min-width:0}
.milestone-dot{position:absolute;top:-40px;left:0;width:26px;height:26px;border-radius:50%;background:var(--accent);border:6px solid var(--bg);box-shadow:0 0 0 2px var(--accent-soft)}
.milestone-title{font-family:var(--font-heading);font-size:28px;font-weight:var(--heading-weight);line-height:1.2;letter-spacing:-0.015em}
.milestone-text{font-size:20px;line-height:1.42;color:var(--muted)}
.image-layout{position:relative;margin:-88px -120px -72px;flex:1}
.image-full{position:absolute;inset:0;width:100%;height:100%;object-fit:cover}
.image-overlay{position:absolute;left:0;right:0;bottom:0;padding:72px 120px 110px;background:linear-gradient(to top,rgba(0,0,0,.8),rgba(0,0,0,0));color:#fff;display:flex;flex-direction:column;gap:16px}
.image-overlay .slide-subtitle{color:rgba(255,255,255,.85)}
.image-overlay .kicker{color:#fff}
.image-overlay .kicker::before{background:#fff}
.slide-footer{position:absolute;left:120px;right:120px;bottom:34px;display:flex;justify-content:space-between;font-size:15px;color:var(--muted);letter-spacing:.14em;text-transform:uppercase;font-weight:500}
.slide-number em{font-style:normal;opacity:.55}
.notes{display:none;position:absolute;left:0;right:0;bottom:0;padding:32px 120px;background:rgba(0,0,0,.86);color:#fff;font-size:26px;line-height:1.4;z-index:3}
body.show-notes .notes{display:block}
.progress{position:fixed;left:0;bottom:0;height:3px;background:linear-gradient(90deg,var(--accent),var(--accent-2));transition:width .5s var(--ease);z-index:5}
.nav{position:fixed;right:28px;bottom:22px;display:flex;gap:8px;z-index:5;opacity:0;transition:opacity .4s var(--ease)}
body.show-nav .nav{opacity:1}
.nav button{width:44px;height:44px;border-radius:50%;border:1px solid rgba(255,255,255,.2);background:rgba(0,0,0,.45);color:#fff;font-size:18px;cursor:pointer;backdrop-filter:blur(10px)}
.nav button:hover{background:rgba(255,255,255,.15)}
.rv{opacity:0;transform:translateY(26px);filter:blur(6px)}
.r-none .rv{opacity:1;transform:none;filter:none}
.r-stagger .slide.active .rv{animation:rv .9s var(--ease) forwards;animation-delay:calc(var(--i,0) * 80ms + 120ms)}
.r-step .slide.active .rv.on{animation:rv .7s var(--ease) forwards}
@keyframes rv{to{opacity:1;transform:none;filter:none}}
.card-icon{width:52px;height:52px;border-radius:14px;background:var(--accent-soft);display:grid;place-items:center;color:var(--accent)}
.card-icon .icon,.step-number .icon{width:28px;height:28px}
.step-number .icon{width:24px;height:24px}
.block-chart{padding:28px 32px 20px;display:flex;flex-direction:column;gap:12px;flex:1;min-height:0}
.chart-title{font-family:var(--font-heading);font-size:26px;font-weight:var(--heading-weight);letter-spacing:-0.01em}
.chart{width:100%;height:100%;min-height:0;flex:1;overflow:visible;--chart-0:var(--accent);--chart-1:var(--accent-2);--chart-2:${hueShift(c.accent, 96)};--chart-3:${hueShift(c.accent, 150)};--chart-4:${hueShift(c.accent, 210)};--chart-5:${hueShift(c.accent, 270)}}
.chart text{font-family:var(--font-body);fill:var(--ink)}
.chart-value{font-size:24px;font-weight:600;font-family:var(--font-heading)}
.chart-label{font-size:20px;fill:var(--muted)}
.chart-legend{font-size:24px}
.chart-pct{fill:var(--muted);font-size:20px}
.chart-total{font-size:56px;font-weight:var(--heading-weight);font-family:var(--font-heading)}
.chart-axis{stroke:var(--glass-border);stroke-width:2}
.chart-line{fill:none;stroke:var(--accent);stroke-width:6;stroke-linecap:round;stroke-linejoin:round}
.chart .dot circle{fill:var(--bg);stroke:var(--accent);stroke-width:5}
.chart .bar rect{transform-origin:center bottom;transform-box:fill-box}
.r-stagger .slide.active .chart .bar rect{animation:grow .9s var(--ease) both;animation-delay:calc(var(--i,0) * 70ms + 300ms)}
.r-stagger .slide.active .chart .arc{animation:sweep 1.1s var(--ease) both;animation-delay:calc(var(--i,0) * 90ms + 300ms)}
.r-stagger .slide.active .chart-line{stroke-dasharray:3000;stroke-dashoffset:3000;animation:draw 1.6s var(--ease) .3s forwards}
@keyframes grow{from{transform:scaleY(0)}to{transform:scaleY(1)}}
@keyframes sweep{from{stroke-dasharray:0 2000}}
@keyframes draw{to{stroke-dashoffset:0}}
.motif{position:absolute;right:-140px;top:50%;transform:translateY(-50%);width:820px;height:820px;color:var(--accent);opacity:${dark ? 0.32 : 0.28};z-index:-1;pointer-events:none}
.motif .fill{fill:currentColor;stroke:none}
.motif .spin{transform-origin:500px 500px;animation:spin 60s linear infinite}
@keyframes spin{to{transform:rotate(360deg)}}
.layout-title .motif,.layout-statement .motif{right:-260px;width:1000px;height:1000px;opacity:${dark ? 0.24 : 0.2}}
.layout-section .motif{right:520px;top:auto;bottom:-300px;transform:none;width:700px;height:700px;opacity:.18}
.variant-accent .motif{color:var(--accent-ink)}
.autofit{zoom:var(--fit,1)}
body.overview .frame{transform:none!important;width:100vw;height:100vh;display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:24px;padding:32px;overflow:auto;align-content:start;box-sizing:border-box}
body.overview .slide{position:relative;inset:auto;opacity:1!important;visibility:visible!important;transform:none!important;transition:none!important;zoom:.22;cursor:pointer;border-radius:24px;outline:6px solid transparent;pointer-events:auto}
body.overview .slide.active{outline-color:var(--accent)}
body.overview .slide .rv{opacity:1!important;transform:none!important;filter:none!important;animation:none!important}
body.overview .progress,body.overview .nav{display:none}
strong{font-weight:700}
code{font-family:var(--font-mono);background:var(--accent-soft);padding:.05em .3em;border-radius:6px;font-size:.92em}
@media (prefers-reduced-motion:reduce){.slide,.blob,.progress{animation:none!important;transition:none!important}.rv{opacity:1!important;transform:none!important;filter:none!important;animation:none!important}}
@media print{html,body{background:#fff;overflow:visible;height:auto}.stage{position:static;display:block;perspective:none}.frame{transform:none!important;width:auto;height:auto}.slide{position:relative;display:flex!important;opacity:1!important;visibility:visible!important;transform:none!important;transition:none!important;page-break-after:always;break-after:page}.rv{opacity:1!important;transform:none!important;filter:none!important}.blob{animation:none!important}.notes,.progress,.nav{display:none!important}@page{size:${SLIDE_WIDTH}px ${SLIDE_HEIGHT}px;margin:0}}
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
    if (location.hash !== '#' + (current + 1)) history.replaceState(null, '', '#' + (current + 1));
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
    var root = slide.querySelector(':scope > .stack, :scope > .split');
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
    if (frame) frame.style.transform = 'scale(' + scale + ')';
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
  if (stage) stage.addEventListener('click', function(e){ if (e.target.closest && e.target.closest('.nav')) return; if (body.classList.contains('overview')) { var target = e.target.closest && e.target.closest('.slide'); if (target) { body.classList.remove('overview'); fit(); show(parseInt(target.getAttribute('data-index'), 10), 1); } return; } var x = e.clientX / window.innerWidth; x < 0.2 ? backward() : forward(); });
  window.addEventListener('hashchange', function(){ show(fromHash(), 1); });
  window.addEventListener('resize', fit);
  window.addEventListener('message', function(e){ var d = e.data || {}; if (d.type === 'slides:goto' && typeof d.index === 'number') show(d.index, d.index > current ? 1 : -1); });
  fit();
  show(fromHash(), 1);
})();
`

export function scriptJson(value: unknown): string {
  return JSON.stringify(value).replace(/</g, '\u003c').replace(/>/g, '\u003e').replace(/&/g, '\u0026')
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

export function renderDeckHtml(deck: Deck, brand?: ThemeOverrides): string {
  const theme = resolveTheme(deck, brand)
  const total = deck.slides.length
  const transition = deck.motion?.transition ?? 'fade'
  const reveal = deck.motion?.reveal ?? 'stagger'
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
<body class="deck t-${transition} r-${reveal}">
<main class="stage"><div class="frame">
${slides}
</div></main>
<div class="progress"></div>
<div class="nav" aria-label="Navigation"><button type="button" class="prev" aria-label="Previous slide">&larr;</button><button type="button" class="next" aria-label="Next slide">&rarr;</button></div>
<script>var DECK_ID=${scriptJson(deck.id)};var NOTES=${scriptJson(deck.slides.map((slide) => slide.notes ?? ''))};var TITLES=${scriptJson(deck.slides.map((slide) => slide.title ?? ''))};var SPEAKER_HTML=${scriptJson(speakerHtml(deck, theme))};</script>
<script>${DECK_SCRIPT}</script>
</body>
</html>
`
}
