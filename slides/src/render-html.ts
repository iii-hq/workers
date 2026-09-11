import type { Block, Deck, Entry, Slide, ThemeOverrides } from './model.js'
import { googleFontsHref, mix, type ResolvedTheme, resolveTheme, withAlpha } from './themes.js'

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
  if (count <= 6) return 3
  if (count <= 9) return 3
  return 4
}

function entries(list: Entry[], render: (entry: Entry, index: number) => string): string {
  return list.map(render).join('')
}

export function renderBlock(block: Block): string {
  switch (block.type) {
    case 'heading':
      return `<h3 class="block block-heading">${inline(block.text)}</h3>`
    case 'text':
      return `<p class="block block-text">${inline(block.text)}</p>`
    case 'bullets':
      return `<ul class="block block-bullets${block.items.length > 5 ? ' dense' : ''}">${block.items.map((item) => `<li>${inline(item)}</li>`).join('')}</ul>`
    case 'image': {
      const src = safeUrl(block.src)
      const img = src
        ? `<img src="${src}" alt="${escapeHtml(block.alt ?? '')}" loading="lazy" />`
        : `<div class="image-missing">Image unavailable</div>`
      return `<figure class="block block-image">${img}${block.caption ? `<figcaption>${inline(block.caption)}</figcaption>` : ''}</figure>`
    }
    case 'code':
      return `<pre class="block block-code"${block.language ? ` data-language="${escapeHtml(block.language)}"` : ''}><code>${escapeHtml(block.code)}</code></pre>`
    case 'quote':
      return `<blockquote class="block block-quote"><p>${inline(block.text)}</p>${block.attribution ? `<cite>${inline(block.attribution)}</cite>` : ''}</blockquote>`
    case 'metric':
      return `<div class="block block-metric"><div class="metric-value">${inline(block.value)}</div><div class="metric-label">${inline(block.label)}</div></div>`
    case 'cards': {
      const cols = gridColumns(block.entries.length)
      const dense = block.entries.length > 6 ? ' dense' : ''
      return `<div class="block block-cards${dense}" style="--cols:${cols}">${entries(
        block.entries,
        (entry, index) =>
          `<div class="card">${block.numbered ? `<div class="card-number">${String(index + 1).padStart(2, '0')}</div>` : ''}<div class="card-title">${inline(entry.title)}</div>${entry.text ? `<div class="card-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    }
    case 'steps':
      return `<div class="block block-steps${block.entries.length > 4 ? ' dense' : ''}">${entries(
        block.entries,
        (entry, index) =>
          `<div class="step"><div class="step-number">${index + 1}</div><div class="step-title">${inline(entry.title)}</div>${entry.text ? `<div class="step-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
    case 'timeline':
      return `<div class="block block-timeline" style="--cols:${Math.max(1, block.entries.length)}">${entries(
        block.entries,
        (entry) =>
          `<div class="milestone"><div class="milestone-dot"></div><div class="milestone-title">${inline(entry.title)}</div>${entry.text ? `<div class="milestone-text">${inline(entry.text)}</div>` : ''}</div>`,
      )}</div>`
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

function kicker(slide: Slide): string {
  return slide.kicker ? `<div class="kicker">${inline(slide.kicker)}</div>` : ''
}

export function renderSlideBody(slide: Slide, index: number, deck: Pick<Deck, 'author'>): string {
  const title = slide.title ? `<h2 class="slide-title">${inline(slide.title)}</h2>` : ''
  const subtitle = slide.subtitle ? `<p class="slide-subtitle">${inline(slide.subtitle)}</p>` : ''
  const blocks = slide.blocks.map(renderBlock).join('')
  const columns = () => {
    const [left, right] = splitColumns(slide.blocks)
    return `<div class="columns"><div class="column">${left.map(renderBlock).join('')}</div><div class="column">${right.map(renderBlock).join('')}</div></div>`
  }
  switch (slide.layout) {
    case 'title':
      return `<div class="stack center hero">${kicker(slide)}${slide.title ? `<h1 class="deck-title">${inline(slide.title)}</h1>` : ''}${subtitle}<div class="hero-meta"><div class="accent-bar"></div>${deck.author ? `<span>${inline(deck.author)}</span>` : ''}</div><div class="blocks">${blocks}</div></div>`
    case 'section':
      return `<div class="stack section"><div class="section-number">${String(index + 1).padStart(2, '0')}</div>${kicker(slide)}${title}${subtitle}<div class="blocks">${blocks}</div></div>`
    case 'statement':
      return `<div class="stack center statement">${kicker(slide)}${title}${subtitle}<div class="blocks">${blocks}</div></div>`
    case 'image': {
      const image = slide.blocks.find((block) => block.type === 'image')
      const rest = slide.blocks
        .filter((block) => block !== image)
        .map(renderBlock)
        .join('')
      const src = image && image.type === 'image' ? safeUrl(image.src) : ''
      return `<div class="stack image-layout">${src ? `<img class="image-full" src="${src}" alt="${escapeHtml(image && image.type === 'image' ? (image.alt ?? '') : '')}" />` : ''}<div class="image-overlay">${kicker(slide)}${title}${subtitle}<div class="blocks">${rest}</div></div></div>`
    }
    case 'two-column':
      return `<div class="stack"><div class="head">${kicker(slide)}${title}${subtitle}</div>${columns()}</div>`
    case 'blank':
      return `<div class="stack"><div class="blocks">${blocks}</div></div>`
    default:
      if (slide.blocks.some((block) => block.column))
        return `<div class="stack"><div class="head">${kicker(slide)}${title}${subtitle}</div>${columns()}</div>`
      return `<div class="stack"><div class="head">${kicker(slide)}${title}${subtitle}</div><div class="blocks">${blocks}</div></div>`
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
  const footer = `<footer class="slide-footer"><span>${theme.footer ? inline(theme.footer) : ''}</span><span class="slide-number">${index + 1} / ${total}</span></footer>`
  const notes = slide.notes ? `<aside class="notes">${inline(slide.notes)}</aside>` : ''
  const variant = slide.variant ?? 'default'
  return `<section class="slide layout-${slide.layout} variant-${variant}" data-index="${index}" id="slide-${index + 1}"${background}><div class="orb orb-a"></div><div class="orb orb-b"></div>${renderSlideBody(slide, index, deck)}${footer}${notes}</section>`
}

export function deckCss(theme: ResolvedTheme): string {
  const c = theme.colors
  const gradientEnd = mix(c.background, c.accent, theme.dark ? 0.35 : 0.18)
  return `
:root{--bg:${c.background};--surface:${c.surface};--ink:${c.ink};--muted:${c.muted};--accent:${c.accent};--accent-ink:${c.accent_ink};--accent-soft:${withAlpha(c.accent, 0.16)};--accent-glow:${withAlpha(c.accent, theme.dark ? 0.35 : 0.22)};--gradient-end:${gradientEnd};--grid:${withAlpha(c.ink, theme.dark ? 0.045 : 0.06)};--font-heading:'${theme.fonts.heading}',ui-sans-serif,system-ui,sans-serif;--font-body:'${theme.fonts.body}',ui-sans-serif,system-ui,sans-serif;--font-mono:'${theme.fonts.mono}',ui-monospace,SFMono-Regular,Menlo,monospace;--radius:${theme.radius}px;--heading-weight:${theme.heading_weight};--w:${SLIDE_WIDTH}px;--h:${SLIDE_HEIGHT}px}
*{box-sizing:border-box}
html,body{margin:0;background:#000;color:var(--ink);font-family:var(--font-body);height:100%}
body.deck{overflow:hidden}
.stage{position:fixed;inset:0;display:grid;place-items:center}
.slide{width:var(--w);height:var(--h);background:var(--bg);color:var(--ink);padding:88px 120px 72px;position:relative;overflow:hidden;background-size:cover;background-position:center;display:none;flex-direction:column;isolation:isolate}
.slide.active{display:flex}
.slide::after{content:'';position:absolute;inset:0;background-image:linear-gradient(var(--grid) 1px,transparent 1px),linear-gradient(90deg,var(--grid) 1px,transparent 1px);background-size:80px 80px;mask-image:radial-gradient(ellipse at 70% 30%,#000 0%,transparent 70%);pointer-events:none;z-index:-1}
.orb{position:absolute;border-radius:50%;filter:blur(90px);pointer-events:none;z-index:-1;opacity:.9}
.orb-a{width:720px;height:720px;right:-220px;top:-300px;background:radial-gradient(circle,var(--accent-glow),transparent 70%)}
.orb-b{width:520px;height:520px;left:-200px;bottom:-260px;background:radial-gradient(circle,var(--accent-soft),transparent 70%)}
.variant-accent{--bg:var(--accent);--ink:var(--accent-ink);--muted:${withAlpha(c.accent_ink, 0.72)};--surface:${withAlpha(c.accent_ink, 0.12)};--accent-soft:${withAlpha(c.accent_ink, 0.2)};--grid:${withAlpha(c.accent_ink, 0.08)}}
.variant-accent .orb-a{background:radial-gradient(circle,${withAlpha('#ffffff', 0.35)},transparent 70%)}
.variant-accent .orb-b{background:radial-gradient(circle,${withAlpha('#000000', 0.3)},transparent 70%)}
.variant-accent .kicker,.variant-accent .metric-value,.variant-accent .section-number,.variant-accent .step-number,.variant-accent .card-number,.variant-accent .milestone-dot{color:var(--accent-ink)}
.variant-accent .accent-bar,.variant-accent .block-bullets li::before,.variant-accent .block-quote,.variant-accent .milestone-dot,.variant-accent .block-timeline::before{background:var(--accent-ink);border-color:var(--accent-ink)}
.variant-accent .step-number{background:${withAlpha(c.accent_ink, 0.18)}}
.variant-gradient{background:linear-gradient(135deg,var(--bg) 0%,var(--gradient-end) 100%)}
.variant-muted{--bg:var(--surface)}
.layout-title .orb-a{width:1100px;height:1100px;right:-380px;top:-420px;opacity:1}
.layout-title .orb-b{width:760px;height:760px;left:-260px;bottom:-380px}
.stack{display:flex;flex-direction:column;gap:28px;flex:1;min-height:0}
.head{display:flex;flex-direction:column;gap:14px;padding-bottom:12px}
.stack.center{justify-content:center;align-items:flex-start;max-width:1240px}
.stack.statement{align-items:center;text-align:center;max-width:none;justify-content:center}
.stack.statement .slide-title{font-size:88px;line-height:1.05;max-width:1300px}
.stack.statement .slide-subtitle{text-align:center;max-width:1000px}
.kicker{display:inline-flex;align-items:center;gap:14px;font-size:20px;font-weight:600;letter-spacing:.18em;text-transform:uppercase;color:var(--accent)}
.kicker::before{content:'';width:28px;height:3px;background:var(--accent);border-radius:2px}
.hero .kicker{margin-bottom:8px}
.hero-meta{display:flex;align-items:center;gap:24px;color:var(--muted);font-size:24px;margin-top:8px}
.accent-bar{width:120px;height:10px;border-radius:999px;background:var(--accent)}
.deck-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:108px;line-height:1;letter-spacing:-0.025em;margin:0;max-width:1240px;text-wrap:balance}
.slide-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:60px;line-height:1.1;letter-spacing:-0.015em;margin:0;text-wrap:balance}
.slide-subtitle{font-size:32px;line-height:1.35;color:var(--muted);margin:0;max-width:1200px}
.section .section-number{font-family:var(--font-heading);font-size:200px;font-weight:var(--heading-weight);color:var(--accent);line-height:.9;opacity:.85;letter-spacing:-0.04em}
.section{justify-content:center}
.section .slide-title{font-size:84px}
.blocks{display:flex;flex-direction:column;gap:28px;min-height:0;flex:1}
.columns{display:grid;grid-template-columns:1fr 1fr;gap:56px;flex:1;min-height:0}
.column{display:flex;flex-direction:column;gap:24px;min-width:0}
.block{margin:0}
.block-heading{font-family:var(--font-heading);font-size:38px;font-weight:var(--heading-weight);line-height:1.2}
.block-text{font-size:30px;line-height:1.45;max-width:1240px}
.block-bullets{font-size:30px;line-height:1.4;padding-left:0;list-style:none;display:flex;flex-direction:column;gap:16px}
.block-bullets.dense{font-size:26px;gap:10px}
.block-bullets li{position:relative;padding-left:44px}
.block-bullets li::before{content:'';position:absolute;left:0;top:.5em;width:14px;height:14px;border-radius:4px;background:var(--accent);transform:rotate(45deg)}
.block-image{display:flex;flex-direction:column;gap:12px;align-items:flex-start;min-height:0;flex:1}
.block-image img{max-width:100%;max-height:100%;object-fit:contain;border-radius:var(--radius);box-shadow:0 24px 60px rgba(0,0,0,.25)}
.block-image figcaption{font-size:22px;color:var(--muted)}
.image-missing{padding:48px;border:2px dashed var(--muted);border-radius:var(--radius);color:var(--muted);font-size:24px}
.block-code{background:var(--surface);border-radius:var(--radius);padding:30px 36px;font-family:var(--font-mono);font-size:23px;line-height:1.5;overflow:hidden;border:1px solid var(--accent-soft);position:relative;white-space:pre-wrap;word-break:break-word}
.block-code[data-language]::before{content:attr(data-language);position:absolute;top:14px;right:20px;font-size:15px;color:var(--muted);text-transform:uppercase;letter-spacing:.08em}
.block-quote{border-left:10px solid var(--accent);padding:12px 0 12px 40px;display:flex;flex-direction:column;gap:18px}
.block-quote p{margin:0;font-family:var(--font-heading);font-size:44px;line-height:1.3;font-style:italic;text-wrap:balance}
.block-quote cite{font-style:normal;font-size:24px;color:var(--muted)}
.block-metric{display:flex;flex-direction:column;gap:8px;padding:30px 36px;background:var(--surface);border-radius:var(--radius);border:1px solid var(--accent-soft);align-self:flex-start;min-width:340px;box-shadow:0 20px 50px rgba(0,0,0,.12)}
.metric-value{font-family:var(--font-heading);font-size:96px;line-height:1;font-weight:var(--heading-weight);color:var(--accent);letter-spacing:-0.03em}
.metric-label{font-size:24px;color:var(--muted)}
.column .block-metric{align-self:stretch}
.block-cards{display:grid;grid-template-columns:repeat(var(--cols,3),minmax(0,1fr));gap:22px;flex:1;min-height:0;align-content:start}
.card{display:flex;flex-direction:column;gap:10px;padding:28px 30px;background:var(--surface);border:1px solid var(--accent-soft);border-radius:var(--radius);min-width:0;position:relative;box-shadow:0 18px 40px rgba(0,0,0,.10)}
.card-number{font-family:var(--font-mono);font-size:18px;letter-spacing:.1em;color:var(--accent);font-weight:600}
.card-title{font-family:var(--font-heading);font-size:30px;font-weight:var(--heading-weight);line-height:1.2;text-wrap:balance}
.card-text{font-size:22px;line-height:1.4;color:var(--muted)}
.block-cards.dense .card{padding:20px 22px;gap:6px}
.block-cards.dense .card-title{font-size:24px}
.block-cards.dense .card-text{font-size:18px}
.block-steps{display:flex;gap:18px;align-items:stretch;flex:1;min-height:0;flex-wrap:wrap}
.step{flex:1 1 0;min-width:200px;display:flex;flex-direction:column;gap:12px;padding:30px 28px;background:var(--surface);border:1px solid var(--accent-soft);border-radius:var(--radius);position:relative}
.step:not(:last-child)::after{content:'\\2192';position:absolute;right:-24px;top:50%;transform:translateY(-50%);width:30px;text-align:center;font-size:30px;color:var(--accent);z-index:1}
.step-number{width:48px;height:48px;border-radius:50%;background:var(--accent-soft);color:var(--accent);display:grid;place-items:center;font-family:var(--font-mono);font-size:22px;font-weight:700}
.step-title{font-family:var(--font-heading);font-size:30px;font-weight:var(--heading-weight);line-height:1.15;text-wrap:balance}
.step-text{font-size:21px;line-height:1.4;color:var(--muted)}
.block-steps.dense .step{padding:22px 20px;gap:8px}
.block-steps.dense .step-title{font-size:24px}
.block-steps.dense .step-text{font-size:18px}
.block-timeline{display:grid;grid-template-columns:repeat(var(--cols,4),minmax(0,1fr));gap:24px;position:relative;padding-top:36px;margin-top:12px}
.block-timeline::before{content:'';position:absolute;left:12px;right:12px;top:12px;height:3px;background:var(--accent-soft);border-radius:2px}
.milestone{display:flex;flex-direction:column;gap:10px;position:relative;min-width:0}
.milestone-dot{position:absolute;top:-36px;left:0;width:26px;height:26px;border-radius:50%;background:var(--accent);border:6px solid var(--bg)}
.milestone-title{font-family:var(--font-heading);font-size:28px;font-weight:var(--heading-weight);line-height:1.2}
.milestone-text{font-size:21px;line-height:1.4;color:var(--muted)}
.image-layout{position:relative;margin:-88px -120px -72px;flex:1}
.image-full{position:absolute;inset:0;width:100%;height:100%;object-fit:cover}
.image-overlay{position:absolute;left:0;right:0;bottom:0;padding:72px 120px 110px;background:linear-gradient(to top,rgba(0,0,0,.78),rgba(0,0,0,0));color:#fff;display:flex;flex-direction:column;gap:16px}
.image-overlay .slide-subtitle{color:rgba(255,255,255,.85)}
.image-overlay .kicker{color:#fff}
.image-overlay .kicker::before{background:#fff}
.slide-footer{position:absolute;left:120px;right:120px;bottom:34px;display:flex;justify-content:space-between;font-size:18px;color:var(--muted);letter-spacing:.04em;text-transform:uppercase}
.notes{display:none;position:absolute;left:0;right:0;bottom:0;padding:32px 120px;background:rgba(0,0,0,.85);color:#fff;font-size:26px;line-height:1.4;z-index:2}
body.show-notes .notes{display:block}
.progress{position:fixed;left:0;bottom:0;height:4px;background:var(--accent);transition:width .25s ease;z-index:2}
strong{font-weight:700}
code{font-family:var(--font-mono);background:var(--accent-soft);padding:.05em .3em;border-radius:6px;font-size:.92em}
@media print{html,body{background:#fff;overflow:visible;height:auto}.stage{position:static;display:block}.slide{display:flex!important;transform:none!important;page-break-after:always;break-after:page}.notes,.progress{display:none!important}@page{size:${SLIDE_WIDTH}px ${SLIDE_HEIGHT}px;margin:0}}
`
}

const DECK_SCRIPT = `
(function(){
  var slides = Array.prototype.slice.call(document.querySelectorAll('.slide'));
  var stage = document.querySelector('.stage');
  var progress = document.querySelector('.progress');
  var total = slides.length;
  var current = 0;
  function fromHash(){ var n = parseInt(location.hash.replace('#','').replace('slide-',''), 10); return isNaN(n) ? 0 : Math.min(total - 1, Math.max(0, n - 1)); }
  function show(i){
    current = Math.min(total - 1, Math.max(0, i));
    slides.forEach(function(s, idx){ s.classList.toggle('active', idx === current); });
    if (progress) progress.style.width = ((current + 1) / total * 100) + '%';
    if (location.hash !== '#' + (current + 1)) history.replaceState(null, '', '#' + (current + 1));
    if (window.parent !== window) window.parent.postMessage({ type: 'slides:navigate', index: current, total: total }, '*');
  }
  function fit(){
    var w = window.innerWidth, h = window.innerHeight;
    var scale = Math.min(w / ${SLIDE_WIDTH}, h / ${SLIDE_HEIGHT});
    slides.forEach(function(s){ s.style.transform = 'scale(' + scale + ')'; s.style.transformOrigin = 'center center'; });
  }
  document.addEventListener('keydown', function(e){
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown' || e.key === ' ' || e.key === 'PageDown') { e.preventDefault(); show(current + 1); }
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp' || e.key === 'PageUp') { e.preventDefault(); show(current - 1); }
    else if (e.key === 'Home') show(0);
    else if (e.key === 'End') show(total - 1);
    else if (e.key === 'n' || e.key === 'N') document.body.classList.toggle('show-notes');
    else if (e.key === 'f' || e.key === 'F') { if (document.fullscreenElement) document.exitFullscreen(); else document.documentElement.requestFullscreen && document.documentElement.requestFullscreen(); }
  });
  if (stage) stage.addEventListener('click', function(e){ var x = e.clientX / window.innerWidth; show(x < 0.25 ? current - 1 : current + 1); });
  window.addEventListener('hashchange', function(){ show(fromHash()); });
  window.addEventListener('resize', fit);
  window.addEventListener('message', function(e){ var d = e.data || {}; if (d.type === 'slides:goto' && typeof d.index === 'number') show(d.index); });
  fit();
  show(fromHash());
})();
`

export function renderDeckHtml(deck: Deck, brand?: ThemeOverrides): string {
  const theme = resolveTheme(deck, brand)
  const total = deck.slides.length
  const slides = deck.slides.map((slide, index) => renderSlide(slide, index, total, theme, deck)).join('\n')
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${escapeHtml(deck.title)}</title>
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="stylesheet" href="${googleFontsHref(theme)}" />
<style>${deckCss(theme)}</style>
</head>
<body class="deck">
<main class="stage">
${slides}
</main>
<div class="progress"></div>
<script>${DECK_SCRIPT}</script>
</body>
</html>
`
}
