// Assemble parts/<Name>.html (template) + logic/<Name>.js (DCLogic) + theme.css
// into <Name>.dc.html artboards, and emit preview/<Name>.html — a local page
// with a mini DC runtime, used only to click-test before publishing.
import { readFileSync, writeFileSync, existsSync, mkdirSync, readdirSync, unlinkSync } from 'node:fs'

const theme = readFileSync('theme.css', 'utf8')
const sizes = JSON.parse(readFileSync('sizes.json', 'utf8'))

const icons = {
  activity: '<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>',
  search: '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
  'chevron-down': '<path d="m6 9 6 6 6-6"/>',
  'chevron-right': '<path d="m9 18 6-6-6-6"/>',
  'arrow-left': '<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>',
  'external-link': '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  settings: '<path d="M20 7h-9"/><path d="M14 17H5"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/>',
  x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
  refresh: '<path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/><path d="M16 16h5v5"/>',
  check: '<path d="M20 6 9 17l-5-5"/>',
  'circle-check': '<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>',
  ban: '<circle cx="12" cy="12" r="10"/><path d="m4.9 4.9 14.2 14.2"/>',
  'rotate-ccw': '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>',
  clock: '<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>',
  commit: '<circle cx="12" cy="12" r="3"/><path d="M3 12h6"/><path d="M15 12h6"/>',
  'file-code': '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z"/><path d="M14 2v6h6"/><path d="m10 13-2 2 2 2"/><path d="m14 17 2-2-2-2"/>',
  alert: '<path d="M10.3 4.3 2.6 18a2 2 0 0 0 1.8 3h15.2a2 2 0 0 0 1.8-3L13.7 4.3a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
  chat: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
  plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
  scan: '<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/><circle cx="12" cy="12" r="3"/><path d="m16 16-1.9-1.9"/>',
  folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
  history: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>',
  info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>',
  bug: '<path d="m8 2 1.88 1.88"/><path d="M14.12 3.88 16 2"/><path d="M9 7.13v-1a3.003 3.003 0 1 1 6 0v1"/><path d="M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6"/><path d="M12 20v-9"/><path d="M6.53 9C4.6 8.8 3 7.1 3 5"/><path d="M6 13H2"/><path d="M3 21c0-2.1 1.7-3.9 3.8-4"/><path d="M20.97 5c0 2.1-1.6 3.8-3.5 4"/><path d="M22 13h-4"/><path d="M17.2 17c2.1.1 3.8 1.9 3.8 4"/>',
  square: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  send: '<path d="M12 19V5"/><path d="m5 12 7-7 7 7"/>',
  inbox: '<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
}

function icon(name, cls = 'i') {
  const body = icons[name]
  if (!body) throw new Error(`unknown icon ${name}`)
  return `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true">${body}</svg>`
}

const expand = (html) =>
  html.replace(/@@icon:([a-z-]+)(?::([a-z ]+))?@@/g, (_, n, cls) => icon(n, cls || 'i'))

const PROPS = {
  Main: {
    theme: { editor: 'enum', options: ['light', 'dark'], default: 'light', section: 'Console' },
  },
}

const shell = (body, logic, props, w, h) => `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <script src="./support.js"></script>
</head>
<body>
<x-dc>
<helmet>
  <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=Geist+Mono:wght@400;500;600&display=swap">
  <style>
${theme}
  </style>
</helmet>
<div class="root" data-theme="{{theme}}" style="position: relative; width: ${w}px; height: ${h}px; overflow: hidden; background: var(--color-bg);">
${body}
</div>
</x-dc>
<script data-dc-script data-props='${JSON.stringify({ ...props, $preview: { width: w, height: h } })}'>
${logic}
</script>
</body>
</html>
`

/* ---------------- local preview (mini DC runtime, never published) --------- */

const MINI_RUNTIME = `
class DCLogic {
  constructor(props) { this.props = props; this.state = {}; }
  setState(patch, cb) {
    const next = typeof patch === 'function' ? patch(this.state) : patch;
    this.state = Object.assign({}, this.state, next);
    window.__render();
    if (cb) cb();
  }
  forceUpdate() { window.__render(); }
}
window.DCLogic = DCLogic;
`

const MINI_RENDERER = `
const HOLE = /^\\s*\\{\\{\\s*([^}]+?)\\s*\\}\\}\\s*$/;
function lookup(scope, path) {
  const p = path.trim();
  if (p === 'true') return true;
  if (p === 'false') return false;
  let v = scope;
  for (const part of p.split('.')) { if (v == null) return undefined; v = v[part]; }
  return v;
}
function subst(text, scope) {
  return text.replace(/\\{\\{\\s*([^}]+?)\\s*\\}\\}/g, (_, p) => { const v = lookup(scope, p); return v == null ? '' : String(v); });
}
function renderInto(src, scope, parent) {
  for (const node of Array.from(src.childNodes)) {
    if (node.nodeType === 3) { const t = subst(node.nodeValue, scope); if (t) parent.appendChild(document.createTextNode(t)); continue; }
    if (node.nodeType !== 1) continue;
    const tag = node.tagName.toLowerCase();
    if (tag === 'helmet') continue;
    if (tag === 'sc-if') {
      const raw = node.getAttribute('value') || '';
      const m = raw.match(HOLE);
      const cond = m ? lookup(scope, m[1]) : raw;
      if (cond) renderInto(node, scope, parent);
      continue;
    }
    if (tag === 'sc-for') {
      const raw = node.getAttribute('list') || '';
      const m = raw.match(HOLE);
      const list = m ? lookup(scope, m[1]) : [];
      const as = node.getAttribute('as') || 'item';
      (list || []).forEach((item, i) => {
        const s2 = Object.assign({}, scope); s2[as] = item; s2['$index'] = i;
        renderInto(node, s2, parent);
      });
      continue;
    }
    const el = node.namespaceURI && node.namespaceURI.includes('svg')
      ? document.createElementNS('http://www.w3.org/2000/svg', tag)
      : document.createElement(tag);
    for (const attr of Array.from(node.attributes)) {
      const name = attr.name;
      const value = attr.value;
      const m = value.match(HOLE);
      if (/^on[a-z]+$/i.test(name) && name.length > 2) {
        if (!m) { console.warn('handler attr is not a whole hole:', name, value); continue; }
        const fn = lookup(scope, m[1]);
        if (typeof fn !== 'function') { console.warn('missing handler', m[1]); continue; }
        let ev = name.slice(2).toLowerCase();
        if (ev === 'change' && tag === 'input') ev = 'input';
        el.addEventListener(ev, fn);
        continue;
      }
      if (name.startsWith('hint-')) continue;
      const out = m ? lookup(scope, m[1]) : subst(value, scope);
      if (name === 'value' && tag === 'input') { el.value = out == null ? '' : out; continue; }
      if (out === false || out == null) continue;
      el.setAttribute(name === 'class' ? 'class' : name, out === true ? '' : String(out));
    }
    renderInto(node, scope, el);
    parent.appendChild(el);
  }
}
const tpl = document.getElementById('dc-template');
const instance = new Component(window.__props);
window.__render = function () {
  const vals = instance.renderVals ? instance.renderVals() : {};
  const scope = Object.assign({}, vals);
  const mount = document.getElementById('dc-mount');
  const active = document.activeElement;
  const focusKey = active && active.getAttribute ? active.getAttribute('data-focus-key') : null;
  const caret = active && active.selectionStart != null ? active.selectionStart : null;
  mount.innerHTML = '';
  renderInto(tpl.content, scope, mount);
  if (focusKey) {
    const again = mount.querySelector('[data-focus-key="' + focusKey + '"]');
    if (again) { again.focus(); if (caret != null && again.setSelectionRange) again.setSelectionRange(caret, caret); }
  }
};
if (instance.componentDidMount) instance.componentDidMount();
window.__render();
`

function preview(name, body, logic, props, w, h) {
  const defaults = Object.fromEntries(Object.entries(props).map(([k, v]) => [k, v.default]))
  return `<!doctype html>
<html><head><meta charset="utf-8">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=Geist+Mono:wght@400;500;600&display=swap">
<style>${theme}</style></head>
<body style="margin:0">
<template id="dc-template">
<div class="root" data-theme="{{theme}}" style="position: relative; width: ${w}px; height: ${h}px; overflow: hidden; background: var(--color-bg);">
${body}
</div>
</template>
<div id="dc-mount"></div>
<script>window.__props = ${JSON.stringify(defaults)};<\/script>
<script>${MINI_RUNTIME}<\/script>
<script>${logic}<\/script>
<script>${MINI_RENDERER}<\/script>
</body></html>
`
}

/* ---------------- build ---------------- */

if (!existsSync('preview')) mkdirSync('preview')
const wanted = new Set(Object.keys(sizes))
for (const f of readdirSync('.')) {
  if (f.endsWith('.dc.html') && !wanted.has(f.replace('.dc.html', ''))) {
    unlinkSync(f)
    console.log(`removed stale ${f}`)
  }
}

for (const [name, [w, h]] of Object.entries(sizes)) {
  const body = expand(readFileSync(`parts/${name}.html`, 'utf8'))
  if (/@@/.test(body)) throw new Error(`unexpanded placeholder in parts/${name}.html`)
  const logicPath = `logic/${name}.js`
  const logic = existsSync(logicPath)
    ? readFileSync(logicPath, 'utf8')
    : 'class Component extends DCLogic {\n  renderVals() {\n    return { theme: this.props.theme ?? \'light\' };\n  }\n}\n'
  const props = PROPS[name] ?? { theme: { editor: 'enum', options: ['light', 'dark'], default: 'light', section: 'Console' } }
  if (/'/.test(JSON.stringify(props))) throw new Error('apostrophe in data-props would break the single-quoted attribute')
  writeFileSync(`${name}.dc.html`, shell(body, logic, props, w, h))
  writeFileSync(`preview/${name}.html`, preview(name, body, logic, props, w, h))
  console.log(`built ${name}.dc.html (${w}x${h})`)
}
