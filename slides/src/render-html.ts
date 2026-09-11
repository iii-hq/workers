import type { Block, Deck, Slide, ThemeOverrides } from './model.js'
import { googleFontsHref, type ResolvedTheme, resolveTheme, withAlpha } from './themes.js'

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

export function renderBlock(block: Block): string {
  switch (block.type) {
    case 'heading':
      return `<h3 class="block block-heading">${inline(block.text)}</h3>`
    case 'text':
      return `<p class="block block-text">${inline(block.text)}</p>`
    case 'bullets':
      return `<ul class="block block-bullets">${block.items.map((item) => `<li>${inline(item)}</li>`).join('')}</ul>`
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
    default:
      return ''
  }
}

function splitColumns(blocks: Block[]): [Block[], Block[]] {
  const left: Block[] = []
  const right: Block[] = []
  blocks.forEach((block, index) => {
    const side = block.column ?? (index % 2 === 0 ? 'left' : 'right')
    ;(side === 'left' ? left : right).push(block)
  })
  return [left, right]
}

export function renderSlideBody(slide: Slide, index: number, total: number): string {
  const title = slide.title ? `<h2 class="slide-title">${inline(slide.title)}</h2>` : ''
  const subtitle = slide.subtitle ? `<p class="slide-subtitle">${inline(slide.subtitle)}</p>` : ''
  const blocks = slide.blocks.map(renderBlock).join('')
  switch (slide.layout) {
    case 'title':
      return `<div class="stack center"><div class="accent-bar"></div>${slide.title ? `<h1 class="deck-title">${inline(slide.title)}</h1>` : ''}${subtitle}<div class="blocks">${blocks}</div></div>`
    case 'section':
      return `<div class="stack section"><div class="section-number">${String(index + 1).padStart(2, '0')}</div>${title}${subtitle}<div class="blocks">${blocks}</div></div>`
    case 'statement':
      return `<div class="stack center statement">${title}${subtitle}<div class="blocks">${blocks}</div></div>`
    case 'image': {
      const image = slide.blocks.find((block) => block.type === 'image')
      const rest = slide.blocks
        .filter((block) => block !== image)
        .map(renderBlock)
        .join('')
      const src = image && image.type === 'image' ? safeUrl(image.src) : ''
      return `<div class="stack image-layout">${src ? `<img class="image-full" src="${src}" alt="${escapeHtml(image && image.type === 'image' ? (image.alt ?? '') : '')}" />` : ''}<div class="image-overlay">${title}${subtitle}<div class="blocks">${rest}</div></div></div>`
    }
    case 'two-column': {
      const [left, right] = splitColumns(slide.blocks)
      return `<div class="stack">${title}${subtitle}<div class="columns"><div class="column">${left.map(renderBlock).join('')}</div><div class="column">${right.map(renderBlock).join('')}</div></div></div>`
    }
    case 'blank':
      return `<div class="stack"><div class="blocks">${blocks}</div></div>`
    default: {
      const hasColumns = slide.blocks.some((block) => block.column)
      if (hasColumns) {
        const [left, right] = splitColumns(slide.blocks)
        return `<div class="stack">${title}${subtitle}<div class="columns"><div class="column">${left.map(renderBlock).join('')}</div><div class="column">${right.map(renderBlock).join('')}</div></div></div>`
      }
      return `<div class="stack">${title}${subtitle}<div class="blocks">${blocks}</div></div>`
    }
  }
}

export function renderSlide(slide: Slide, index: number, total: number, theme: ResolvedTheme): string {
  const background = slide.background
    ? /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(slide.background)
      ? ` style="background:${escapeHtml(slide.background)}"`
      : safeUrl(slide.background)
        ? ` style="background-image:url('${safeUrl(slide.background)}')"`
        : ''
    : ''
  const footer = `<footer class="slide-footer"><span>${theme.footer ? inline(theme.footer) : ''}</span><span class="slide-number">${index + 1} / ${total}</span></footer>`
  const notes = slide.notes ? `<aside class="notes">${inline(slide.notes)}</aside>` : ''
  return `<section class="slide layout-${slide.layout}" data-index="${index}" id="slide-${index + 1}"${background}>${renderSlideBody(slide, index, total)}${footer}${notes}</section>`
}

export function deckCss(theme: ResolvedTheme): string {
  const c = theme.colors
  return `
:root{--bg:${c.background};--surface:${c.surface};--ink:${c.ink};--muted:${c.muted};--accent:${c.accent};--accent-ink:${c.accent_ink};--accent-soft:${withAlpha(c.accent, 0.16)};--font-heading:'${theme.fonts.heading}',ui-sans-serif,system-ui,sans-serif;--font-body:'${theme.fonts.body}',ui-sans-serif,system-ui,sans-serif;--font-mono:'${theme.fonts.mono}',ui-monospace,SFMono-Regular,Menlo,monospace;--radius:${theme.radius}px;--heading-weight:${theme.heading_weight};--w:${SLIDE_WIDTH}px;--h:${SLIDE_HEIGHT}px}
*{box-sizing:border-box}
html,body{margin:0;background:#000;color:var(--ink);font-family:var(--font-body);height:100%}
body.deck{overflow:hidden}
.stage{position:fixed;inset:0;display:grid;place-items:center}
.slide{width:var(--w);height:var(--h);background:var(--bg);color:var(--ink);padding:96px 120px 72px;position:relative;overflow:hidden;background-size:cover;background-position:center;display:none;flex-direction:column}
.slide.active{display:flex}
.stack{display:flex;flex-direction:column;gap:32px;flex:1;min-height:0}
.stack.center{justify-content:center;align-items:flex-start;max-width:1200px}
.stack.statement{align-items:center;text-align:center;max-width:none;justify-content:center}
.stack.statement .slide-title{font-size:88px;line-height:1.05}
.accent-bar{width:120px;height:12px;border-radius:999px;background:var(--accent)}
.deck-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:104px;line-height:1.02;letter-spacing:-0.02em;margin:0}
.slide-title{font-family:var(--font-heading);font-weight:var(--heading-weight);font-size:64px;line-height:1.1;letter-spacing:-0.015em;margin:0}
.slide-subtitle{font-size:34px;line-height:1.35;color:var(--muted);margin:0;max-width:1200px}
.section .section-number{font-family:var(--font-heading);font-size:160px;font-weight:var(--heading-weight);color:var(--accent);line-height:1;opacity:.9}
.section .slide-title{font-size:84px}
.blocks{display:flex;flex-direction:column;gap:28px;min-height:0}
.columns{display:grid;grid-template-columns:1fr 1fr;gap:64px;flex:1;min-height:0}
.column{display:flex;flex-direction:column;gap:28px;min-width:0}
.block{margin:0}
.block-heading{font-family:var(--font-heading);font-size:40px;font-weight:var(--heading-weight);line-height:1.2}
.block-text{font-size:32px;line-height:1.45}
.block-bullets{font-size:32px;line-height:1.45;padding-left:0;list-style:none;display:flex;flex-direction:column;gap:14px}
.block-bullets li{position:relative;padding-left:44px}
.block-bullets li::before{content:'';position:absolute;left:0;top:.55em;width:16px;height:16px;border-radius:5px;background:var(--accent);transform:rotate(45deg)}
.block-image{display:flex;flex-direction:column;gap:12px;align-items:flex-start;min-height:0;flex:1}
.block-image img{max-width:100%;max-height:100%;object-fit:contain;border-radius:var(--radius);box-shadow:0 24px 60px rgba(0,0,0,.25)}
.block-image figcaption{font-size:22px;color:var(--muted)}
.image-missing{padding:48px;border:2px dashed var(--muted);border-radius:var(--radius);color:var(--muted);font-size:24px}
.block-code{background:var(--surface);border-radius:var(--radius);padding:32px 36px;font-family:var(--font-mono);font-size:24px;line-height:1.5;overflow:hidden;border:1px solid var(--accent-soft);position:relative;white-space:pre-wrap;word-break:break-word}
.block-code[data-language]::before{content:attr(data-language);position:absolute;top:14px;right:20px;font-size:16px;color:var(--muted);text-transform:uppercase;letter-spacing:.08em}
.block-quote{border-left:10px solid var(--accent);padding:12px 0 12px 40px;display:flex;flex-direction:column;gap:18px}
.block-quote p{margin:0;font-family:var(--font-heading);font-size:44px;line-height:1.3;font-style:italic}
.block-quote cite{font-style:normal;font-size:26px;color:var(--muted)}
.block-metric{display:flex;flex-direction:column;gap:8px;padding:28px 36px;background:var(--surface);border-radius:var(--radius);border:1px solid var(--accent-soft);align-self:flex-start;min-width:320px}
.metric-value{font-family:var(--font-heading);font-size:96px;line-height:1;font-weight:var(--heading-weight);color:var(--accent)}
.metric-label{font-size:26px;color:var(--muted)}
.column .block-metric{align-self:stretch}
.image-layout{position:relative;margin:-96px -120px -72px;flex:1}
.image-full{position:absolute;inset:0;width:100%;height:100%;object-fit:cover}
.image-overlay{position:absolute;left:0;right:0;bottom:0;padding:72px 120px 110px;background:linear-gradient(to top,rgba(0,0,0,.72),rgba(0,0,0,0));color:#fff;display:flex;flex-direction:column;gap:20px}
.image-overlay .slide-subtitle{color:rgba(255,255,255,.85)}
.slide-footer{position:absolute;left:120px;right:120px;bottom:36px;display:flex;justify-content:space-between;font-size:20px;color:var(--muted);letter-spacing:.02em}
.notes{display:none;position:absolute;left:0;right:0;bottom:0;padding:32px 120px;background:rgba(0,0,0,.85);color:#fff;font-size:26px;line-height:1.4}
body.show-notes .notes{display:block}
.progress{position:fixed;left:0;bottom:0;height:4px;background:var(--accent);transition:width .25s ease;z-index:2}
strong{font-weight:700}
code{font-family:var(--font-mono);background:var(--accent-soft);padding:.05em .3em;border-radius:6px;font-size:.92em}
@media print{html,body{background:#fff;overflow:visible;height:auto}.stage{position:static;display:block}.slide{display:flex!important;transform:none!important;page-break-after:always;break-after:page}.notes,.progress{display:none!important}@page{size:${SLIDE_WIDTH}px ${SLIDE_HEIGHT}px;margin:0}}
`
}

const DECK_SCRIPT = String.raw`
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
  const slides = deck.slides.map((slide, index) => renderSlide(slide, index, total, theme)).join('\n')
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
