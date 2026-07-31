// OpenWiki browser UI. Single-page app served inline by the worker.
// No npm deps; vanilla HTML/CSS/JS. Mermaid loads on demand from a CDN for the
// diagram view and falls back to rendering the source when blocked.

export const INDEX_HTML = String.raw`<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>openwiki</title>
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link href="https://fonts.googleapis.com/css2?family=Chivo+Mono:wght@400;500;600&display=swap" rel="stylesheet" />
<style>
  :root{
    --bg:#f2f0ed; --panel:#e9e6e2; --panel-2:#ebe8e3; --border:#d8d5d0;
    --rule-2:#e6e3df; --text:#0a0a0a; --text-dim:#6b6865; --ink-ghost:#a3a09c;
    --accent:#b8420f; --accent-2:#b8420f; --accent-fg:#f2f0ed;
    --danger:#c43e1c; --warn:#a87a00;
  }
  [data-theme="dark"]{
    --bg:#111110; --panel:#1a1916; --panel-2:#1f1e1c; --border:#2a2926;
    --rule-2:#1f1e1c; --text:#f2f0ed; --text-dim:#a8a49e; --ink-ghost:#8a8782;
    --accent:#3ea8ff; --accent-2:#3ea8ff; --accent-fg:#111110;
    --danger:#c43e1c; --warn:#a87a00;
  }
  *{box-sizing:border-box}
  html,body{margin:0;padding:0;height:100%;background:var(--bg);color:var(--text);
    font-family:'Chivo Mono',ui-monospace,SFMono-Regular,Menlo,monospace;
    font-feature-settings:'liga' 0,'calt' 0;
    font-size:13px;line-height:1.6;-webkit-font-smoothing:antialiased}
  a{color:var(--accent);text-decoration:none}
  a:hover{text-decoration:underline}
  button,input{font:inherit;color:inherit}
  button{cursor:pointer}

  #app{display:grid;grid-template-rows:48px 1fr;grid-template-columns:280px minmax(0,1fr) 244px;
    grid-template-areas:"top top top" "side main rail";height:100vh}
  @media (max-width:1100px){#app{grid-template-columns:280px minmax(0,1fr);
    grid-template-areas:"top top" "side main"} .rail{display:none}}

  /* right rail: on-this-page + relevant sources + copy + freshness */
  .rail{grid-area:rail;border-left:1px solid var(--border);background:var(--bg);
    overflow-y:auto;padding:22px 18px;display:flex;flex-direction:column;gap:22px}
  .rail.empty-rail{display:none}
  @media (max-width:1100px){.rail{display:none}}
  .rail-sec h4{margin:0 0 10px;font-size:10px;text-transform:uppercase;letter-spacing:.14em;
    color:var(--ink-ghost);font-weight:600}
  .rail a{display:block;color:var(--text-dim);font-size:12px;padding:3px 0;line-height:1.45;
    white-space:nowrap;overflow:hidden;text-overflow:ellipsis;border-left:1px solid transparent;padding-left:10px;margin-left:-1px}
  .rail a:hover{color:var(--text);border-left-color:var(--accent)}
  .rail a.h3{padding-left:22px;font-size:11.5px}
  .rail .src{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11.5px}
  .rail .fresh{font-size:11.5px;color:var(--text-dim);display:flex;align-items:center;gap:6px}
  .copymd{background:transparent;border:1px solid var(--border);color:var(--text-dim);
    padding:8px 10px;font-size:11px;text-transform:lowercase;letter-spacing:.06em;width:100%;text-align:left;border-radius:0}
  .copymd:hover{border-color:var(--accent);color:var(--text)}

  /* nested nav tree */
  .nav-folder{margin-top:4px}
  .nav-folder>.fhead{display:flex;align-items:center;gap:6px;padding:5px 4px;cursor:pointer;
    color:var(--text);font-size:12.5px;font-weight:600;user-select:none}
  .nav-folder>.fhead .caret{font-size:9px;opacity:.55;transition:transform .15s}
  .nav-folder.collapsed>.fhead .caret{transform:rotate(-90deg)}
  .nav-folder.collapsed>.fchildren{display:none}
  .fchildren{margin-left:10px;border-left:1px solid var(--border);padding-left:8px}
  .nav-leaf{padding:4px 8px;border-radius:0;cursor:pointer;color:var(--text-dim);font-size:12.5px;
    white-space:nowrap;overflow:hidden;text-overflow:ellipsis;border:1px solid transparent;outline:none}
  .nav-leaf:hover{color:var(--text)}
  .nav-leaf.active{color:var(--accent);border-color:var(--accent)}
  .topbar{grid-area:top;display:flex;align-items:center;gap:14px;padding:0 18px;
    background:var(--bg);border-bottom:1px solid var(--border)}
  .brand{display:flex;align-items:center;gap:9px;font-weight:600;letter-spacing:.3px;font-size:13px}
  .brand-dot{width:8px;height:8px;border-radius:0;background:var(--accent)}
  .breadcrumb{color:var(--text-dim);font-size:12.5px;flex:0 1 auto;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .breadcrumb .sep{margin:0 8px;opacity:.5}
  .top-spacer{flex:1}
  .wiki-actions{display:none;align-items:center;gap:8px}
  .wiki-actions.on{display:flex}
  .search{position:relative}
  .search input{width:240px;background:transparent;border:1px solid var(--border);
    border-radius:0;padding:6px 10px;color:var(--text);outline:none;transition:border .15s}
  .search input:focus{border-color:var(--accent)}
  .search input::placeholder{color:var(--ink-ghost)}
  .tbtn{background:transparent;border:1px solid var(--border);border-radius:0;
    padding:6px 14px;color:var(--text-dim);text-transform:lowercase;letter-spacing:.04em}
  .tbtn:hover:not(:disabled){border-color:var(--accent);color:var(--text)}
  .tbtn:disabled{opacity:.4;cursor:not-allowed}
  .cites{margin-top:28px;border-top:1px solid var(--border);padding-top:14px}
  .cites h3{font-size:11px;text-transform:uppercase;letter-spacing:.07em;
    color:var(--text-dim);font-weight:600;margin:0 0 8px}
  .cites a,.cites span.nolink{display:inline-block;margin:2px 10px 2px 0;font-size:12px;
    font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
  .cites span.nolink{color:var(--text-dim)}
  .modal.wide{max-width:780px;width:92vw}
  .ask-input{width:100%;background:var(--panel-2);border:1px solid var(--border);
    border-radius:6px;padding:9px 11px;color:var(--text);outline:none}
  .ask-input:focus{border-color:var(--accent)}
  .ask-answer{margin-top:16px;max-height:52vh;overflow:auto}
  .ask-answer .md{font-size:13px}
  .mermaid-box{background:var(--panel-2);border:1px solid var(--border);border-radius:6px;
    padding:14px;overflow:auto;text-align:center;margin-top:6px}
  .mermaid-box pre{text-align:left;white-space:pre-wrap;margin:0}
  .mermaid-render{margin:1.2em 0}
  .mermaid-render .mmout{background:var(--panel-2);border:1px solid var(--border);
    padding:16px;overflow-x:auto;text-align:center}
  .mermaid-render .mmout svg{max-width:100%;height:auto}
  .mermaid-render .mmout pre{text-align:left;white-space:pre-wrap;margin:0;font-size:12px}

  .sidebar{grid-area:side;background:var(--panel);border-right:1px solid var(--border);
    overflow-y:auto;padding:16px;display:flex;flex-direction:column;gap:20px}
  .side-section h3{margin:0 0 8px;font-size:11px;text-transform:uppercase;
    letter-spacing:.08em;color:var(--text-dim);font-weight:600}
  .newwiki{display:flex;flex-direction:column;gap:8px}
  .newwiki input{background:transparent;border:1px solid var(--border);
    border-radius:0;padding:8px 10px;outline:none;color:var(--text)}
  .newwiki input::placeholder{color:var(--ink-ghost)}
  .newwiki input:focus{border-color:var(--accent)}
  .newwiki button{background:var(--accent);color:var(--accent-fg);border:1px solid var(--accent);
    border-radius:0;padding:8px 12px;font-weight:600;text-transform:lowercase;letter-spacing:.04em}
  .newwiki button:disabled{background:transparent;color:var(--ink-ghost);border-color:var(--border);cursor:not-allowed}
  .model-select{background:transparent;border:1px solid var(--border);border-radius:0;
    padding:7px 8px;color:var(--text);outline:none;font-size:12px}
  .model-select:focus{border-color:var(--accent)}
  .model-select:disabled{color:var(--ink-ghost);opacity:.6}
  .model-hint{font-size:11px;color:var(--ink-ghost);line-height:1.45;display:none}
  .model-hint.show{display:block}
  .model-hint a{color:var(--accent)}

  .wikilist{display:flex;flex-direction:column;gap:1px}
  .wiki-item{padding:9px 10px;border-radius:0;cursor:pointer;
    display:flex;flex-direction:column;gap:3px;border:1px solid transparent;border-left:2px solid transparent}
  .wiki-item:hover{background:var(--panel)}
  .wiki-item.active{background:var(--panel);border-left-color:var(--accent)}
  .wiki-item .wiki-head{display:flex;align-items:center;gap:8px}
  .wiki-item .wiki-head .name{flex:1;min-width:0}
  .wiki-item .name{font-weight:500;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .wiki-del{flex:none;border:none;background:none;color:var(--text-dim);font-size:15px;line-height:1;
    cursor:pointer;padding:0 3px;opacity:0;transition:opacity .12s,color .12s}
  .wiki-item:hover .wiki-del{opacity:.55}
  .wiki-del:hover,.wiki-del:focus,.wiki-del:focus-visible{opacity:1;color:var(--danger)}
  @media (hover:none){.wiki-del{opacity:.55}}
  .wiki-sched{margin-top:4px;width:100%;font-size:10.5px;color:var(--text-dim);background:var(--panel);
    border:1px solid var(--border);border-radius:0;padding:2px 4px;font-family:inherit;cursor:pointer;
    text-transform:lowercase;letter-spacing:.03em;opacity:0;transition:opacity .12s}
  .wiki-item:hover .wiki-sched,.wiki-sched:focus{opacity:.9}
  @media (hover:none){.wiki-sched{opacity:.9}}
  .wiki-item .meta{font-size:11px;color:var(--text-dim);font-variant-numeric:tabular-nums}
  .wiki-item .genrow{display:flex;align-items:center;gap:7px;font-size:11px;color:var(--accent);
    text-transform:lowercase;letter-spacing:.04em;font-variant-numeric:tabular-nums}
  .wiki-item .genrow.error{color:var(--danger)}
  .wiki-item .progressbar{margin-top:5px;height:3px;background:var(--rule-2);overflow:hidden}
  .wiki-item .progressbar > div{height:100%;background:var(--accent);transition:width .3s}

  .cat{margin-top:6px}
  .cat-head{display:flex;align-items:center;gap:6px;padding:6px 4px;cursor:pointer;
    color:var(--text-dim);font-size:12px;text-transform:uppercase;letter-spacing:.05em;
    font-weight:600;user-select:none}
  .cat-head .caret{display:inline-block;transition:transform .15s;font-size:9px;opacity:.6}
  .cat.collapsed .caret{transform:rotate(-90deg)}
  .cat.collapsed .pages{display:none}
  .pages{display:flex;flex-direction:column;gap:1px;margin-left:2px}
  .page-item{padding:5px 10px;border-radius:5px;cursor:pointer;color:var(--text);
    white-space:nowrap;overflow:hidden;text-overflow:ellipsis;outline:none;
    border:1px solid transparent}
  .page-item:hover{background:var(--panel-2)}
  .page-item.active{background:var(--panel-2);color:var(--accent);border-color:var(--border)}
  .page-item:focus{border-color:var(--accent)}
  .page-snippet{font-size:11px;color:var(--text-dim);
    white-space:nowrap;overflow:hidden;text-overflow:ellipsis;padding:0 10px 4px}

  .main{grid-area:main;overflow-y:auto;padding:32px 48px 64px;max-width:900px;width:100%}
  .page-head{margin-bottom:24px;border-bottom:1px solid var(--border);padding-bottom:16px}
  .page-title{margin:0 0 8px;font-size:28px;font-weight:700;letter-spacing:-.01em}
  .chips{display:flex;flex-wrap:wrap;gap:6px}
  .chip{background:var(--panel-2);border:1px solid var(--border);
    padding:2px 8px;border-radius:99px;font-size:11px;color:var(--text-dim)}
  .chip.cat{color:var(--accent)}

  .md h1,.md h2,.md h3,.md h4{margin:1.6em 0 .6em;font-weight:600;line-height:1.3}
  .md h1{font-size:1.8em;border-bottom:1px solid var(--border);padding-bottom:.3em}
  .md h2{font-size:1.4em}
  .md h3{font-size:1.15em}
  .md h4{font-size:1em;color:var(--text-dim)}
  .md p{margin:.7em 0}
  .md ul,.md ol{margin:.6em 0;padding-left:1.6em}
  .md li{margin:.2em 0}
  .md code{background:var(--panel-2);border:1px solid var(--border);
    padding:1px 5px;border-radius:4px;font-size:.9em;font-family:ui-monospace,SFMono-Regular,Menlo,monospace}
  .md pre{background:var(--panel-2);border:1px solid var(--border);border-radius:6px;
    padding:12px 14px;overflow-x:auto;margin:.8em 0}
  .md pre code{background:transparent;border:0;padding:0;font-size:.88em;line-height:1.5}
  .md blockquote{border-left:3px solid var(--accent);
    background:rgba(124,156,255,.06);margin:.8em 0;padding:.4em 12px;color:var(--text-dim)}
  .md hr{border:0;border-top:1px solid var(--border);margin:1.5em 0}
  .md img{max-width:100%;border-radius:6px}
  .md a{color:var(--accent)}
  .md .tablewrap{overflow-x:auto;margin:1.1em 0;border:1px solid var(--border)}
  .md table{border-collapse:collapse;width:100%;font-size:12.5px}
  .md th,.md td{border-bottom:1px solid var(--rule-2);border-right:1px solid var(--rule-2);
    padding:7px 12px;text-align:left;vertical-align:top;line-height:1.5}
  .md th:last-child,.md td:last-child{border-right:0}
  .md tbody tr:last-child td{border-bottom:0}
  .md thead th{background:var(--panel);font-weight:600;border-bottom:1px solid var(--border);
    text-transform:uppercase;letter-spacing:.06em;font-size:11px;color:var(--text-dim)}

  .hero{display:flex;flex-direction:column;align-items:center;justify-content:center;
    text-align:center;height:100%;color:var(--text-dim);gap:12px}
  .hero h1{margin:0;font-size:28px;color:var(--text)}
  .hero p{margin:0;max-width:420px}

  .banner{position:fixed;top:60px;left:50%;transform:translateX(-50%);
    background:rgba(255,107,107,.14);border:1px solid var(--danger);
    color:#ffbcbc;padding:8px 14px;border-radius:6px;font-size:13px;z-index:100}

  /* iii motion vocabulary (storybook.iii.dev): no spin. busy = pulse ring,
     opacity pulse, hard blink, gradient shimmer. */
  .spinner{display:inline-block;width:6px;height:6px;border-radius:9999px;
    background:var(--accent);vertical-align:middle;animation:pulse-dot 1.6s ease-out infinite}
  .dot{display:inline-block;width:6px;height:6px;border-radius:9999px;flex-shrink:0;background:var(--accent)}
  .dot.pulse{animation:pulse-dot 1.6s ease-out infinite}
  @keyframes pulse-dot{0%{box-shadow:0 0 0 0 var(--accent)}to{box-shadow:0 0 0 8px transparent}}
  @keyframes blink{0%,49%{opacity:1}50%,100%{opacity:0}}
  @keyframes ow-shimmer{0%{background-position:200% 0}100%{background-position:-200% 0}}
  @keyframes pulse-op{50%{opacity:.5}}
  .caret{display:inline-block;width:6px;height:13px;background:var(--text);vertical-align:middle;
    margin-left:2px;animation:blink 1s step-end infinite}
  .shimmer{background-image:linear-gradient(90deg,var(--ink-ghost),var(--text) 50%,var(--ink-ghost));
    background-size:200% 100%;-webkit-background-clip:text;background-clip:text;color:transparent;
    animation:ow-shimmer 2.4s linear infinite}
  .pulse-op{animation:pulse-op 2s cubic-bezier(.4,0,.6,1) infinite}

  /* live generation console (storybook install-progress pattern) */
  .owgen{width:100%;max-width:640px;border:1px solid var(--border);background:var(--bg);text-align:left}
  .owgen .head{background:var(--panel);border-bottom:1px solid var(--rule-2);padding:7px 14px;
    font-size:11px;text-transform:uppercase;letter-spacing:.12em;color:var(--ink-ghost);
    display:flex;align-items:center;gap:8px}
  .owgen .body{padding:16px 14px;display:flex;flex-direction:column;gap:14px}
  .owgen .owbar{display:flex;height:6px;width:100%;overflow:hidden;background:var(--rule-2)}
  .owgen .owbar>i{display:block;height:100%;background:var(--accent);transition:width .3s}
  .owgen .owbar>i.fail{background:var(--danger)}
  .owgen .barlabel{display:flex;justify-content:space-between;font-size:11px;color:var(--text-dim);
    text-transform:lowercase;letter-spacing:.04em;font-variant-numeric:tabular-nums}
  .owgen .console{background:var(--bg);font-size:12.5px;line-height:1.6;display:flex;flex-direction:column;gap:3px}
  .owgen .stage{display:flex;align-items:baseline;gap:8px;color:var(--text-dim)}
  .owgen .stage.done{color:var(--text)} .owgen .stage.fail{color:var(--danger)}
  .owgen .stage .glyph{color:var(--ink-ghost);width:10px;display:inline-block}
  .owgen .stage.done .glyph{color:var(--accent)}
  .owgen .stage .pct{margin-left:auto;color:var(--ink-ghost);font-variant-numeric:tabular-nums}
  .owgen .working{color:var(--ink-ghost);font-size:12.5px}
  .owgen .lastpage{font-size:13px;line-height:1.7;color:var(--text-dim);font-style:italic}
  .owgen .owfeed{border-top:1px solid var(--rule-2);margin-top:6px;padding-top:8px;
    max-height:260px;overflow-y:auto;display:flex;flex-direction:column;gap:2px;
    font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11.5px;line-height:1.5}
  .owgen .feedln{color:var(--text-dim);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .owgen .feedln.hi{color:var(--accent)}
  .owgen .feedln .fg{color:var(--ink-ghost);display:inline-block;width:12px}
  .owgen .feedln.hi .fg{color:var(--accent)}

  .phase{font-size:11px;color:var(--accent);text-transform:lowercase;letter-spacing:.04em;margin-top:2px}
  .phase.error{color:var(--danger)}

  .footer{padding:16px 0 0;border-top:1px solid var(--border);margin-top:auto;
    font-size:12px;color:var(--text-dim)}
  .footer a{color:var(--text-dim)}
  .footer a:hover{color:var(--accent)}

  .modal-back{position:fixed;inset:0;background:rgba(0,0,0,.6);
    display:flex;align-items:center;justify-content:center;z-index:200}
  .modal{background:var(--panel);border:1px solid var(--border);border-radius:10px;
    max-width:520px;padding:24px}
  .modal h2{margin:0 0 10px}
  .modal button{margin-top:14px;background:var(--panel-2);border:1px solid var(--border);
    color:var(--text);border-radius:6px;padding:6px 12px}

  .empty{color:var(--text-dim);font-size:12px;padding:8px 4px}

  /* --- drafting-sheet enforcement (the law) --- */
  /* radii: only 0 or full. boxes -> 0; dots keep their round. */
  input,button,.chip,.modal,.wiki-item,.page-item,.tbtn,.search input,
  .newwiki input,.newwiki button,.ask-input,.mermaid-box,.banner,
  .cites a,.cites span.nolink{border-radius:0}
  /* single rationed accent: no gradients, no glows, no shadows. */
  .brand-dot{background:var(--accent);box-shadow:none}
  .newwiki button:disabled{background:transparent;color:var(--ink-ghost);filter:none}
  .wiki-item .progressbar>div{background:var(--accent)}
  /* active = accent left-rule, never a fill flood. */
  .page-item.active{color:var(--accent)}
  .hero h1{background:none;-webkit-text-fill-color:var(--text);color:var(--text)}
  .modal,.banner{box-shadow:none}
  .md a,a{color:var(--accent)}
  /* lowercase chrome; generated content (.md, .page-title) keeps its case. */
  .brand,.side-section h3,.cat-head,.tbtn,.newwiki button,.footer,.breadcrumb,
  .wiki-item .meta,.empty,.phase,.cites h3{text-transform:lowercase}
</style>
</head>
<body>
<div id="app">
  <div class="topbar">
    <div class="brand"><span class="brand-dot"></span>OpenWiki</div>
    <div class="breadcrumb" id="breadcrumb"></div>
    <div class="top-spacer"></div>
    <div class="wiki-actions" id="wiki-actions">
      <div class="search">
        <input id="search" type="search" placeholder="search this wiki" disabled />
      </div>
      <button class="tbtn" id="ask-btn" disabled>ask</button>
    </div>
    <button class="tbtn" id="theme-btn">dark</button>
  </div>
  <aside class="sidebar">
    <div class="side-section">
      <h3>New wiki</h3>
      <form class="newwiki" id="newwiki-form">
        <input id="repo-url" placeholder="https://github.com/owner/repo" autocomplete="off" />
        <select id="model-select" class="model-select" title="generation model"></select>
        <div class="model-hint" id="model-hint"></div>
        <button type="submit" id="gen-btn" disabled>Generate</button>
      </form>
    </div>
    <div class="side-section">
      <h3>Wikis</h3>
      <div class="wikilist" id="wikilist"><div class="empty">Loading\u2026</div></div>
    </div>
    <div class="side-section" id="pages-section" style="display:none">
      <h3>Pages</h3>
      <div id="pagelist"></div>
    </div>
    <div class="footer">
      <a href="#" id="about-link">about openwiki</a>
    </div>
  </aside>
  <main class="main" id="main"></main>
  <aside class="rail empty-rail" id="rail"></aside>
</div>

<script>
(() => {
  'use strict';

  //--- state
  const state = {
    wikis: [],
    currentWikiId: null,
    currentWiki: null,      // full detail
    pages: [],              // pages for current wiki
    pagesByCat: new Map(),  // catId -> [pages]
    currentSlug: null,
    searchResults: null,    // null = show pages, [] = show empty results
    generating: new Map(),  // wiki_id -> {phase, progress, message}
    pollTimers: new Map(),
    collapsedCats: new Set(),
    collapsedNav: new Set(),
  };

  const tocSlug = (s) => String(s || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
  function extractToc(md) {
    const out = [];
    for (const line of String(md || '').split('\n')) {
      const m = line.match(/^(#{2,3})\s+(.+)$/);
      if (!m) continue;
      const text = m[2].replace(/[\`*]/g, '').trim();
      out.push({ level: m[1].length, text, id: tocSlug(text) });
    }
    return out.slice(0, 24);
  }
  function timeAgo(ts) {
    const s = Math.max(0, (Date.now() - new Date(ts).getTime()) / 1000);
    if (s < 60) return Math.floor(s) + 's ago';
    if (s < 3600) return Math.floor(s / 60) + 'm ago';
    if (s < 86400) return Math.floor(s / 3600) + 'h ago';
    if (s < 604800) return Math.floor(s / 86400) + 'd ago';
    return new Date(ts).toLocaleDateString();
  }

  // Inline mermaid: render fenced mermaid blocks in place (CDN, lazy).
  let _mermaid = null;
  let _mmSeq = 0;
  async function loadMermaid() {
    if (_mermaid) return _mermaid;
    const mod = await import('https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs');
    _mermaid = mod.default;
    const dark = document.documentElement.getAttribute('data-theme') === 'dark';
    _mermaid.initialize({ startOnLoad: false, theme: dark ? 'dark' : 'default', securityLevel: 'strict' });
    return _mermaid;
  }
  function mermaidSourceFallback(box) {
    const src = box.querySelector('.mmsrc');
    const out = box.querySelector('.mmout');
    if (!src || !out) return;
    out.textContent = '';
    const pre = document.createElement('pre');
    pre.textContent = src.textContent;
    out.appendChild(pre);
  }
  async function renderMermaidBlocks(root) {
    const blocks = root.querySelectorAll('.mermaid-render');
    if (!blocks.length) return;
    let mermaid;
    try { mermaid = await loadMermaid(); } catch { blocks.forEach(mermaidSourceFallback); return; }
    for (const box of blocks) {
      const src = box.querySelector('.mmsrc');
      const out = box.querySelector('.mmout');
      if (!src || !out) continue;
      try {
        const { svg } = await mermaid.render('mm' + (++_mmSeq) + '-' + Date.now(), src.textContent);
        out.innerHTML = svg;
      } catch { mermaidSourceFallback(box); }
    }
  }

  //--- helpers
  const $ = (id) => document.getElementById(id);
  const el = (tag, attrs, ...kids) => {
    const n = document.createElement(tag);
    if (attrs) for (const k in attrs) {
      if (k === 'class') n.className = attrs[k];
      else if (k === 'text') n.textContent = attrs[k];
      else if (k.startsWith('on')) n.addEventListener(k.slice(2), attrs[k]);
      else if (attrs[k] === true) n.setAttribute(k, '');
      else if (attrs[k] != null && attrs[k] !== false) n.setAttribute(k, attrs[k]);
    }
    for (const k of kids) {
      if (k == null || k === false) continue;
      n.appendChild(typeof k === 'string' ? document.createTextNode(k) : k);
    }
    return n;
  };
  const escapeHtml = (s) => String(s).replace(/[&<>"']/g, (c) => ({
    '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'
  })[c]);

  async function api(path, opts) {
    const r = await fetch('/openwiki/api' + path, opts);
    let body = null;
    try { body = await r.json(); } catch(_) {}
    if (!r.ok) {
      const msg = (body && (body.error || body.message)) || (r.status + ' ' + r.statusText);
      throw new Error(msg);
    }
    return body;
  }

  let bannerTimer = null;
  function flashError(msg) {
    const existing = document.querySelector('.banner');
    if (existing) existing.remove();
    if (bannerTimer) clearTimeout(bannerTimer);
    const b = el('div', { class:'banner', text: msg });
    document.body.appendChild(b);
    bannerTimer = setTimeout(() => b.remove(), 6000);
  }

  //--- markdown renderer (self-contained, safe)
  function renderMarkdown(src, opts) {
    const wikiId = opts && opts.wikiId;
    const lines = String(src || '').replace(/\r\n?/g, '\n').split('\n');
    let out = '';
    let i = 0;

    // Inline rendering with HTML escaping.
    function inline(text) {
      // Extract code spans first (protect from other rules).
      const spans = [];
      let t = text.replace(/\`([^\`\n]+)\`/g, (_, code) => {
        spans.push('<code>' + escapeHtml(code) + '</code>');
        return '\u0001' + (spans.length - 1) + '\u0001';
      });
      // Escape remaining html.
      t = escapeHtml(t);
      // Images: ![alt](src)
      t = t.replace(/!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g, (_, alt, src) => {
        if (!/^(https?:|data:|\/)/i.test(src)) return '';
        return '<img alt="' + escapeHtml(alt) + '" src="' + escapeHtml(src) + '" />';
      });
      // Links: [text](url)
      t = t.replace(/\[([^\]]+)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g, (_, label, url) => {
        let href = url;
        let target = '';
        if (/^https?:/i.test(url)) {
          target = ' target="_blank" rel="noopener noreferrer"';
        } else if (/^(\.\/)?([\w\-]+)\.md$/i.test(url) && wikiId) {
          const m = url.match(/([\w\-]+)\.md$/i);
          href = '#/wiki/' + wikiId + '/page/' + m[1];
        } else if (url.startsWith('#')) {
          href = url;
        } else {
          // relative/unknown -> keep as text
          return escapeHtml(label);
        }
        return '<a href="' + escapeHtml(href) + '"' + target + '>' + label + '</a>';
      });
      // Bold **x** then italic *x*.
      t = t.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
      t = t.replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<em>$2</em>');
      // Restore code spans.
      t = t.replace(/\u0001(\d+)\u0001/g, (_, idx) => spans[+idx]);
      return t;
    }

    function closeLists(stack) {
      let s = '';
      while (stack.length) s += '</li></' + stack.pop().type + '>';
      return s;
    }

    const listStack = []; // {type:'ul'|'ol', indent}
    let para = [];
    function flushPara() {
      if (para.length) {
        out += '<p>' + inline(para.join(' ')) + '</p>';
        para = [];
      }
    }

    while (i < lines.length) {
      const line = lines[i];

      // fenced code block
      const fence = line.match(/^\`\`\`\s*([\w.+-]*)\s*$/);
      if (fence) {
        flushPara();
        out += closeLists(listStack);
        const lang = fence[1] || '';
        const buf = [];
        i++;
        while (i < lines.length && !/^\`\`\`\s*$/.test(lines[i])) { buf.push(lines[i]); i++; }
        i++; // consume closing fence
        if (lang === 'mermaid') {
          out += '<div class="mermaid-render"><pre class="mmsrc" hidden>' + escapeHtml(buf.join('\n')) + '</pre><div class="mmout"><div class="spinner"></div></div></div>';
        } else {
          const cls = lang ? ' class="lang-' + escapeHtml(lang) + '"' : '';
          out += '<pre><code' + cls + '>' + escapeHtml(buf.join('\n')) + '</code></pre>';
        }
        continue;
      }

      // blank line
      if (/^\s*$/.test(line)) {
        flushPara();
        out += closeLists(listStack);
        i++;
        continue;
      }

      // heading
      const h = line.match(/^(#{1,4})\s+(.*)$/);
      if (h) {
        flushPara();
        out += closeLists(listStack);
        const lvl = h[1].length;
        const hid = tocSlug(h[2].replace(/[\`*]/g, '').trim());
        out += '<h' + lvl + ' id="' + escapeHtml(hid) + '">' + inline(h[2].trim()) + '</h' + lvl + '>';
        i++;
        continue;
      }

      // hr
      if (/^\s{0,3}(---+|\*\*\*+|___+)\s*$/.test(line)) {
        flushPara();
        out += closeLists(listStack);
        out += '<hr/>';
        i++;
        continue;
      }

      // blockquote (consume run)
      if (/^\s*>\s?/.test(line)) {
        flushPara();
        out += closeLists(listStack);
        const buf = [];
        while (i < lines.length && /^\s*>\s?/.test(lines[i])) {
          buf.push(lines[i].replace(/^\s*>\s?/, ''));
          i++;
        }
        out += '<blockquote>' + inline(buf.join(' ')) + '</blockquote>';
        continue;
      }

      // GFM table: a header row with pipes, then a separator row (---, with
      // optional :align:). Without this, pipe rows fall through to a paragraph
      // and render as a raw "| a | b |" blob.
      if (line.includes('|') && i + 1 < lines.length
          && /-/.test(lines[i + 1]) && /^\s*\|?[\s:|-]+\|?\s*$/.test(lines[i + 1])) {
        flushPara();
        out += closeLists(listStack);
        const splitRow = (r) => r.trim().replace(/^\|/, '').replace(/\|$/, '').split(/(?<!\\)\|/).map((c) => c.replace(/\\\|/g, '|').trim());
        const header = splitRow(line);
        const aligns = splitRow(lines[i + 1]).map((c) => {
          const l = c.startsWith(':'), rt = c.endsWith(':');
          return l && rt ? 'center' : rt ? 'right' : l ? 'left' : '';
        });
        i += 2;
        const rows = [];
        while (i < lines.length && lines[i].includes('|') && !/^\s*$/.test(lines[i])) { rows.push(splitRow(lines[i])); i++; }
        const cell = (tag, text, ci) => '<' + tag + (aligns[ci] ? ' style="text-align:' + aligns[ci] + '"' : '') + '>' + inline(text || '') + '</' + tag + '>';
        let tbl = '<div class="tablewrap"><table><thead><tr>';
        header.forEach((c, ci) => { tbl += cell('th', c, ci); });
        tbl += '</tr></thead><tbody>';
        for (const row of rows) {
          tbl += '<tr>';
          for (let ci = 0; ci < header.length; ci++) tbl += cell('td', row[ci], ci);
          tbl += '</tr>';
        }
        out += tbl + '</tbody></table></div>';
        continue;
      }

      // list item
      const li = line.match(/^(\s*)([-*+]|\d+\.)\s+(.*)$/);
      if (li) {
        flushPara();
        const indent = li[1].length;
        const type = /\d/.test(li[2]) ? 'ol' : 'ul';
        // pop deeper lists
        while (listStack.length && listStack[listStack.length - 1].indent > indent) {
          out += '</li></' + listStack.pop().type + '>';
        }
        // same-level: close previous li
        const top = listStack[listStack.length - 1];
        if (top && top.indent === indent) {
          if (top.type !== type) {
            out += '</li></' + listStack.pop().type + '>';
            out += '<' + type + '>';
            listStack.push({ type, indent });
          } else {
            out += '</li>';
          }
        } else if (!top || top.indent < indent) {
          out += '<' + type + '>';
          listStack.push({ type, indent });
        }
        out += '<li>' + inline(li[3]);
        i++;
        continue;
      }

      // paragraph accumulate
      para.push(line.trim());
      i++;
    }
    flushPara();
    out += closeLists(listStack);
    return out;
  }

  //--- URL hash routing
  function parseHash() {
    const h = location.hash || '';
    const m = h.match(/^#\/wiki\/([^\/]+)(?:\/page\/(.+))?$/);
    if (!m) return { wikiId: null, slug: null };
    return { wikiId: m[1], slug: m[2] || null };
  }
  function setHash(wikiId, slug) {
    const target = '#/wiki/' + wikiId + (slug ? '/page/' + slug : '');
    if (location.hash !== target) {
      history.replaceState(null, '', target);
    }
  }

  //--- wiki list
  async function loadWikis() {
    try {
      state.wikis = await api('/wikis');
    } catch (e) {
      flashError('Failed to load wikis: ' + e.message);
      state.wikis = [];
    }
    renderWikiList();
  }

  // Populate the generation model picker from the router's live catalog, grouped
  // by provider. Credentials stay in llm-router; when no provider is configured
  // the catalog is empty and we point the user at the console to set one up.
  async function loadModels() {
    const sel = $('model-select');
    const hint = $('model-hint');
    if (!sel) return;
    let data = { models: [], default_model: '' };
    try { data = await api('/models'); } catch (_) { /* router absent */ }
    const models = data.models || [];
    sel.textContent = '';
    if (!models.length) {
      sel.style.display = 'none';
      if (hint) {
        hint.className = 'model-hint show';
        hint.textContent = 'No model provider configured. Set one up in the console chat, then reload — pages fall back to a heuristic build until then.';
      }
      return;
    }
    if (hint) hint.className = 'model-hint';
    sel.style.display = '';
    const byProvider = new Map();
    for (const m of models) {
      if (!byProvider.has(m.provider)) byProvider.set(m.provider, []);
      byProvider.get(m.provider).push(m);
    }
    for (const [provider, list] of byProvider) {
      const group = el('optgroup', { label: provider });
      for (const m of list) group.appendChild(el('option', { value: m.id, text: m.id }));
      sel.appendChild(group);
    }
    const def = data.default_model;
    if (def && models.some((m) => m.id === def)) sel.value = def;
  }

  function renderWikiList() {
    const root = $('wikilist');
    root.textContent = '';
    if (!state.wikis.length && !state.generating.size) {
      root.appendChild(el('div', { class:'empty', text:'No wikis yet.' }));
      return;
    }
    // generating entries first
    for (const [wid, gen] of state.generating.entries()) {
      if (state.wikis.find((w) => w.id === wid)) continue;
      root.appendChild(renderWikiItem({ id: wid, repo_name: gen.repo_name || wid, page_count: 0 }, gen));
    }
    for (const w of state.wikis) {
      const gen = state.generating.get(w.id);
      root.appendChild(renderWikiItem(w, gen));
    }
  }

  function renderWikiItem(w, gen) {
    const generating = gen && gen.phase && gen.phase !== 'ready';
    const err = generating && gen.phase === 'error';
    const node = el('div', {
      class: 'wiki-item' + (state.currentWikiId === w.id ? ' active' : ''),
      onclick: () => selectWiki(w.id),
    });
    const head = el('div', { class:'wiki-head' }, el('div', { class:'name', text: w.repo_name || w.id }));
    // Delete control: hover-revealed X per wiki. Hidden while generating (stop it
    // first). stopPropagation so removing does not also select the wiki.
    if (!generating) {
      head.appendChild(el('button', {
        class: 'wiki-del', title: 'Remove this wiki', 'aria-label': 'Remove this wiki',
        onclick: (e) => { e.stopPropagation(); deleteWiki(w.id); },
      }, '×'));
    }
    node.appendChild(head);
    // While generating, the stored page_count is stale (0 until finalize). Show
    // the live count from the progress stream instead of a confusing "0 pages".
    if (generating && !err) {
      const total = gen.pages_total || 0;
      const done = gen.pages_done || 0;
      const count = total ? (done + ' / ' + total + ' pages') : (gen.phase + '\u2026');
      node.appendChild(el('div', { class:'meta', text: count }));
    } else {
      node.appendChild(el('div', { class:'meta',
        text: (w.page_count || 0) + ' pages' + (w.category_count ? ' \u00b7 ' + w.category_count + ' categories' : '') }));
      node.appendChild(scheduleControl(w));
    }
    if (generating) {
      const pct = Math.round((gen.progress || 0) * 100);
      const row = el('div', { class:'genrow' + (err ? ' error' : '') });
      if (!err) row.appendChild(el('span', { class:'dot pulse' }));
      row.appendChild(el('span', { text: err ? ('error: ' + (gen.message || 'failed')) : (gen.phase + ' \u00b7 ' + pct + '%') }));
      node.appendChild(row);
      if (!err) node.appendChild(el('div', { class:'progressbar' }, el('div', { style: 'width:' + pct + '%' })));
    }
    return node;
  }

  // Per-wiki auto-refresh cadence. A small select, so the schedule is set from
  // the UI (never hardcoded). Clicks/changes stopPropagation so touching it does
  // not select the wiki.
  function scheduleControl(w) {
    const cur = w.refresh_schedule || 'off';
    const opts = [['off', 'off'], ['3h', 'every 3h'], ['6h', 'every 6h'], ['12h', 'every 12h'], ['daily', 'daily'], ['weekly', 'weekly']];
    const sel = el('select', {
      class: 'wiki-sched', title: 'Auto-refresh cadence', 'aria-label': 'Auto-refresh cadence',
      onclick: (e) => e.stopPropagation(),
      onchange: (e) => { e.stopPropagation(); setSchedule(w.id, e.target.value); },
    });
    for (const [val, label] of opts) {
      sel.appendChild(el('option', { value: val, text: 'auto-refresh: ' + label, ...(cur === val ? { selected: true } : {}) }));
    }
    return sel;
  }

  async function setSchedule(id, schedule) {
    try {
      await api('/wikis/' + id + '/schedule', { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ schedule }) });
    } catch (e) { flashError('Could not set auto-refresh: ' + (e.message || e)); return; }
    const w = state.wikis.find((x) => x.id === id);
    if (w) w.refresh_schedule = schedule;
  }

  async function deleteWiki(id) {
    try { await api('/wikis/' + id, { method: 'DELETE' }); }
    catch (e) { flashError('Could not remove wiki: ' + (e.message || e)); return; }
    state.wikis = state.wikis.filter((w) => w.id !== id);
    state.generating.delete(id);
    if (state.currentWikiId === id) {
      state.currentWikiId = null; state.currentSlug = null; state.currentWiki = null;
      state.pages = []; location.hash = '';
      $('wiki-actions').classList.remove('on');
      renderMain(null); renderPageList(); renderBreadcrumb();
    }
    renderWikiList();
  }

  //--- generation flow
  async function generateWiki(repoUrl) {
    const sel = $('model-select');
    const model = sel && sel.value ? sel.value : undefined;
    let resp;
    try {
      resp = await api('/wikis', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ repo_url: repoUrl, ...(model ? { model } : {}) }),
      });
    } catch (e) {
      flashError('Generate failed: ' + e.message);
      return;
    }
    const wid = resp.wiki_id;
    state.generating.set(wid, { phase: resp.status || 'cloning', progress: 0, repo_name: repoUrl.split('/').slice(-1)[0] });
    renderWikiList();
    pollStatus(wid);
  }

  // Live generation progress. Prefer SSE push (real-time); fall back to polling.
  function pollStatus(wid) {
    if (state.pollTimers.has(wid)) return;
    const cur = () => state.generating.get(wid) || {};
    const applyStatus = (evt) => {
      const c = cur();
      state.generating.set(wid, {
        phase: evt.phase, progress: evt.progress || 0, message: evt.message || evt.error,
        pages_done: evt.pages_done != null ? evt.pages_done : c.pages_done,
        pages_total: evt.pages_total != null ? evt.pages_total : c.pages_total,
        lastPage: c.lastPage, repo_name: c.repo_name,
      });
      renderWikiList();
      if (wid === state.currentWikiId && evt.phase !== 'ready') renderMainProgress(state.generating.get(wid));
    };
    const finish = (evt) => {
      if (evt.phase === 'error') {
        if (wid === state.currentWikiId) renderMainProgress(cur());
        flashError('Generation failed: ' + (evt.error || evt.message || 'unknown'));
      } else {
        state.generating.delete(wid);
        loadWikis().then(() => selectWiki(wid));
      }
    };

    if (typeof EventSource !== 'undefined') {
      let es = null;
      try { es = new EventSource('/openwiki/api/wikis/' + wid + '/events'); } catch (_) { es = null; }
      if (es) {
        const stop = () => { try { es.close(); } catch (_) { /* ignore */ } state.pollTimers.delete(wid); };
        state.pollTimers.set(wid, { stop });
        const pushFeed = (g, op, text) => {
          if (!g.feed) g.feed = [];
          g.feed.push({ op, text });
          if (g.feed.length > 40) g.feed.shift();
        };
        es.onmessage = (m) => {
          let evt; try { evt = JSON.parse(m.data); } catch (_) { return; }
          if (evt.kind === 'page') {
            const g = cur(); g.lastPage = evt.title || evt.slug; g.message = 'wrote ' + (evt.title || evt.slug);
            pushFeed(g, 'page', 'wrote page: ' + (evt.title || evt.slug));
            state.generating.set(wid, g);
            if (wid === state.currentWikiId) renderMainProgress(g);
            return;
          }
          if (evt.kind === 'spawn') {
            const g = cur();
            pushFeed(g, 'spawn', 'spawned page-writer: ' + (evt.slug || evt.title || ''));
            state.generating.set(wid, g);
            if (wid === state.currentWikiId) renderMainProgress(g);
            return;
          }
          if (evt.kind === 'activity') {
            const g = cur(); const line = evt.op + (evt.path ? ' ' + evt.path : '');
            g.activity = line;
            pushFeed(g, evt.op, line);
            state.generating.set(wid, g);
            if (wid === state.currentWikiId) renderMainProgress(g);
            return;
          }
          applyStatus(evt);
          if (evt.phase === 'ready' || evt.phase === 'error' || evt.final) { stop(); finish(evt); }
        };
        es.onerror = () => { /* EventSource auto-reconnects; server re-seeds on connect */ };
        return;
      }
    }

    // Poll fallback.
    const stopPoll = () => { const h = state.pollTimers.get(wid); if (h && h.timer) clearInterval(h.timer); state.pollTimers.delete(wid); };
    const tick = async () => {
      let st;
      try { st = await api('/wikis/' + wid + '/status'); }
      catch (e) { state.generating.set(wid, { phase: 'error', progress: 0, message: e.message }); renderWikiList(); stopPoll(); return; }
      applyStatus(st);
      if (st.phase === 'ready' || st.phase === 'error') { stopPoll(); finish(st); }
    };
    const timer = setInterval(tick, 1500);
    state.pollTimers.set(wid, { timer, stop: stopPoll });
    tick();
  }

  //--- select wiki / page
  async function selectWiki(id, slugHint) {
    if (state.currentWikiId !== id) {
      state.currentWikiId = id;
      state.currentWiki = null;
      state.pages = [];
      state.pagesByCat = new Map();
      state.currentSlug = null;
      state.searchResults = null;
    }
    renderWikiList();
    setHash(id, slugHint || null);
    $('search').disabled = false;
    $('ask-btn').disabled = false;
    $('wiki-actions').classList.add('on');
    try {
      const [wiki, pages] = await Promise.all([
        api('/wikis/' + id),
        api('/wikis/' + id + '/pages'),
      ]);
      state.currentWiki = wiki;
      state.pages = pages;
      groupPages();
    } catch (e) {
      flashError('Failed to load wiki: ' + e.message);
      return;
    }
    renderBreadcrumb();
    renderPageList();
    // still building: show live progress and poll until ready.
    if (state.currentWiki && state.currentWiki.generating) {
      pollStatus(id);
      renderMainProgress(state.generating.get(id) || { phase: 'generating', progress: 0 });
      return;
    }
    // pick a page: honor an explicit slug, else land on the reading-flow start
    // (the first leaf of the ordered nav = Getting Started), NOT pages[0] which is
    // write order and can be API Reference.
    let target = slugHint;
    if (!target || !state.pages.find((p) => p.slug === target)) {
      const firstNav = firstNavSlug(state.currentWiki && state.currentWiki.navigation);
      const prefer = state.pages.find((p) => p.slug === 'overview')
        || state.pages.find((p) => p.slug === 'readme')
        || (firstNav && state.pages.find((p) => p.slug === firstNav))
        || state.pages[0];
      target = prefer && prefer.slug;
    }
    if (target) selectPage(target);
    else renderMain(null);
  }

  function firstNavSlug(nav) {
    for (const n of nav || []) {
      if (n.slug) return n.slug;
      if (n.children) { const s = firstNavSlug(n.children); if (s) return s; }
    }
    return null;
  }

  function groupPages() {
    const byCat = new Map();
    const cats = (state.currentWiki && state.currentWiki.categories) || [];
    for (const c of cats) byCat.set(c.id, []);
    for (const p of state.pages) {
      const cid = p.category || 'uncategorized';
      if (!byCat.has(cid)) byCat.set(cid, []);
      byCat.get(cid).push(p);
    }
    state.pagesByCat = byCat;
  }

  function renderBreadcrumb() {
    const b = $('breadcrumb');
    b.textContent = '';
    if (!state.currentWiki) return;
    b.appendChild(el('span', { text: state.currentWiki.repo_name || state.currentWiki.id }));
    if (state.currentSlug) {
      const p = state.pages.find((x) => x.slug === state.currentSlug);
      if (p) {
        b.appendChild(el('span', { class:'sep', text:'/' }));
        b.appendChild(el('span', { text: p.title || p.slug }));
      }
    }
  }

  function renderPageList() {
    const root = $('pagelist');
    root.textContent = '';
    $('pages-section').style.display = state.currentWikiId ? '' : 'none';
    if (!state.currentWikiId) return;

    if (state.searchResults !== null) {
      if (!state.searchResults.length) {
        root.appendChild(el('div', { class:'empty', text:'No results.' }));
        return;
      }
      for (const r of state.searchResults) {
        const item = el('div', {
          class: 'page-item' + (r.slug === state.currentSlug ? ' active' : ''),
          tabindex: '0',
          onclick: () => selectPage(r.slug),
          onkeydown: (e) => pageKey(e, r.slug),
        }, r.title || r.slug);
        root.appendChild(item);
        if (r.snippet) root.appendChild(el('div', { class:'page-snippet', text: r.snippet }));
      }
      return;
    }

    const nav = (state.currentWiki && state.currentWiki.navigation) || null;
    if (nav && nav.length) {
      for (const node of nav) root.appendChild(renderNavNode(node, 0));
      return;
    }

    const cats = (state.currentWiki && state.currentWiki.categories) || [];
    const seen = new Set();
    for (const c of cats) {
      seen.add(c.id);
      root.appendChild(renderCategory(c.id, c.title, state.pagesByCat.get(c.id) || []));
    }
    // uncategorized / extras
    for (const [cid, arr] of state.pagesByCat.entries()) {
      if (seen.has(cid)) continue;
      root.appendChild(renderCategory(cid, cid, arr));
    }
  }

  function renderNavNode(node, depth) {
    const hasChildren = node.children && node.children.length;
    if (node.slug && !hasChildren) {
      return el('div', {
        class: 'nav-leaf' + (node.slug === state.currentSlug ? ' active' : ''),
        tabindex: '0',
        onclick: () => selectPage(node.slug),
        onkeydown: (e) => pageKey(e, node.slug),
      }, node.title || node.slug);
    }
    const key = 'nav:' + depth + ':' + (node.title || '');
    const collapsed = state.collapsedNav.has(key);
    const folder = el('div', { class: 'nav-folder' + (collapsed ? ' collapsed' : '') });
    folder.appendChild(el('div', {
      class: 'fhead',
      onclick: () => { if (state.collapsedNav.has(key)) state.collapsedNav.delete(key); else state.collapsedNav.add(key); renderPageList(); },
    }, el('span', { class: 'caret', text: '▼' }), el('span', { text: node.title || '' })));
    const kids = el('div', { class: 'fchildren' });
    if (node.slug) kids.appendChild(renderNavNode({ title: 'Overview', slug: node.slug }, depth + 1));
    for (const c of node.children || []) kids.appendChild(renderNavNode(c, depth + 1));
    folder.appendChild(kids);
    return folder;
  }

  function renderCategory(cid, title, pages) {
    const collapsed = state.collapsedCats.has(cid);
    const node = el('div', { class: 'cat' + (collapsed ? ' collapsed' : '') });
    const head = el('div', { class:'cat-head',
      onclick: () => {
        if (state.collapsedCats.has(cid)) state.collapsedCats.delete(cid);
        else state.collapsedCats.add(cid);
        renderPageList();
      },
    },
      el('span', { class:'caret', text: '\u25BC' }),
      el('span', { text: title }),
    );
    node.appendChild(head);
    const ul = el('div', { class:'pages' });
    for (const p of pages) {
      ul.appendChild(el('div', {
        class: 'page-item' + (p.slug === state.currentSlug ? ' active' : ''),
        tabindex: '0',
        'data-slug': p.slug,
        onclick: () => selectPage(p.slug),
        onkeydown: (e) => pageKey(e, p.slug),
      }, p.title || p.slug));
    }
    node.appendChild(ul);
    return node;
  }

  function pageKey(e, slug) {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); selectPage(slug); return; }
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const items = Array.from(document.querySelectorAll('#pagelist .nav-leaf, #pagelist .page-item'));
    const idx = items.findIndex((n) => n === e.target);
    if (idx < 0) return;
    const next = items[idx + (e.key === 'ArrowDown' ? 1 : -1)];
    if (next) next.focus();
  }

  async function selectPage(slug) {
    if (!state.currentWikiId) return;
    state.currentSlug = slug;
    setHash(state.currentWikiId, slug);
    renderPageList();
    renderBreadcrumb();
    renderMain('loading');
    try {
      const page = await api('/wikis/' + state.currentWikiId + '/pages/' + encodeURIComponent(slug));
      renderMain(page);
    } catch (e) {
      flashError('Failed to load page: ' + e.message);
      renderMain(null);
    }
  }

  function renderMainProgress(gen) {
    const g = gen || { phase: 'queued', progress: 0 };
    const err = g.phase === 'error';
    const pct = Math.round((g.progress || 0) * 100);
    const done = g.pages_done || 0;
    const total = g.pages_total || 0;
    const order = ['cloning', 'inventorying', 'planning', 'generating'];
    const label = {
      cloning: 'clone repository', inventorying: 'read source files',
      planning: 'plan structure', generating: total ? ('write pages ' + done + '/' + total) : 'write pages',
    };
    let cur = order.indexOf(g.phase);
    if (cur < 0) cur = 0;
    const stages = order.map((ph, i) => {
      let st = '';
      if (i < cur) st = 'done';
      else if (i === cur) st = err ? 'fail' : 'run';
      const glyph = st === 'done' ? '✓' : st === 'fail' ? '×' : st === 'run' ? '→' : '·';
      const cls = st === 'done' ? ' done' : st === 'fail' ? ' fail' : '';
      const pctTag = (st === 'run' && ph === 'generating' && total)
        ? '<span class="pct">' + Math.round((done / total) * 100) + '%</span>' : '';
      return '<div class="stage' + cls + '"><span class="glyph">' + glyph + '</span><span>' + label[ph] + '</span>' + pctTag + '</div>';
    }).join('');
    const working = err
      ? '<div class="stage fail"><span class="glyph">×</span><span>' + escapeHtml(g.message || 'generation failed') + '</span></div>'
      : (g.activity
        ? '<div class="working"><span class="glyph" style="color:var(--accent)">→</span> <span class="shimmer">' + escapeHtml(g.activity) + '</span></div>'
        : '<div class="working pulse-op">· working…</div>');
    // Live activity feed: every file read, sub-agent spawn, and page written,
    // streamed from the SSE events as they happen.
    const feedGlyph = { read: '→', list: '☰', grep: '⌕', spawn: '↳', page: '✓' };
    const feed = (!err && g.feed && g.feed.length)
      ? '<div class="owfeed">' + g.feed.slice(-16).map((f) => {
        const hi = (f.op === 'spawn' || f.op === 'page') ? ' hi' : '';
        return '<div class="feedln' + hi + '"><span class="fg">' + (feedGlyph[f.op] || '·') + '</span> ' + escapeHtml(f.text) + '</div>';
      }).join('') + '</div>' : '';
    const failBar = err ? '<i class="fail" style="width:' + Math.max(pct, 6) + '%"></i>' : '';
    $('main').innerHTML =
      '<div class="hero">' +
        '<div class="owgen">' +
          '<div class="head">' + (err ? '<span class="dot" style="background:var(--danger)"></span>' : '<span class="dot pulse"></span>') +
            (err ? 'generation interrupted' : 'generating wiki') + '</div>' +
          '<div class="body">' +
            '<div class="owbar">' + (err ? failBar : '<i style="width:' + pct + '%"></i>') + '</div>' +
            '<div class="barlabel"><span>' + escapeHtml(g.phase || 'working') + '</span><span>' + pct + '%</span></div>' +
            '<div class="console">' + stages + '</div>' +
            feed +
            working +
          '</div>' +
        '</div>' +
      '</div>';
    renderRail(null);
  }

  function renderMain(page) {
    const root = $('main');
    root.textContent = '';
    if (page === 'loading') {
      root.appendChild(el('div', { class:'hero' }, el('div', { class:'spinner' })));
      renderRail(null);
      return;
    }
    if (!page) {
      renderRail(null);
      if (!state.wikis.length && !state.generating.size) {
        root.appendChild(el('div', { class:'hero' },
          el('h1', { text:'OpenWiki' }),
          el('p', { text:'Enter a repo URL to get started.' }),
          el('p', { text:'OpenWiki generates a browsable knowledge base from any source repository.' }),
        ));
      } else {
        root.appendChild(el('div', { class:'hero' },
          el('p', { text:'Select a wiki from the left to browse it.' })));
      }
      return;
    }
    const head = el('div', { class:'page-head' });
    head.appendChild(el('h1', { class:'page-title', text: page.title || page.slug }));
    const chips = el('div', { class:'chips' });
    if (page.category) chips.appendChild(el('span', { class:'chip cat', text: page.category }));
    if (page.status && page.status !== 'current') chips.appendChild(el('span', { class:'chip', text: page.status }));
    head.appendChild(chips);
    root.appendChild(head);
    const body = el('div', { class:'md' });
    body.innerHTML = renderMarkdown(page.markdown || '', { wikiId: state.currentWikiId });
    root.appendChild(body);
    renderMermaidBlocks(body);
    if (page.citations && page.citations.length) root.appendChild(renderCitations(page.citations));
    renderRail(page);
  }

  function renderRail(page) {
    const rail = $('rail');
    if (!rail) return;
    rail.textContent = '';
    if (!page || page === 'loading') { rail.classList.add('empty-rail'); return; }
    rail.classList.remove('empty-rail');
    const wiki = state.currentWiki;
    if (wiki) {
      const sec = el('div', { class:'rail-sec' });
      if (wiki.repo_url) sec.appendChild(el('a', { class:'src', href: wiki.repo_url, target:'_blank', rel:'noopener noreferrer', text: wiki.repo_name || wiki.repo_url }));
      if (wiki.updated_at) sec.appendChild(el('div', { class:'fresh', text: 'updated ' + timeAgo(wiki.updated_at) }));
      rail.appendChild(sec);
    }
    const toc = extractToc(page.markdown || '');
    if (toc.length) {
      const sec = el('div', { class:'rail-sec' }, el('h4', { text:'On this page' }));
      for (const t of toc) {
        sec.appendChild(el('a', {
          href: '#' + t.id, class: t.level === 3 ? 'h3' : '', text: t.text,
          onclick: (e) => { e.preventDefault(); const n = document.getElementById(t.id); if (n) n.scrollIntoView({ behavior:'smooth', block:'start' }); },
        }));
      }
      rail.appendChild(sec);
    }
    const srcs = page.source_paths || [];
    if (srcs.length) {
      const sec = el('div', { class:'rail-sec' }, el('h4', { text:'Relevant source files' }));
      for (const sp of srcs.slice(0, 14)) {
        const cite = (page.citations || []).find((c) => c.path === sp && c.url && /^https?:\/\//i.test(c.url));
        if (cite) sec.appendChild(el('a', { class:'src', href: cite.url, target:'_blank', rel:'noopener noreferrer', text: sp }));
        else sec.appendChild(el('div', { class:'src', style:'color:var(--text-dim);padding:2px 0', text: sp }));
      }
      rail.appendChild(sec);
    }
    const btn = el('button', { class:'copymd', text:'copy markdown' });
    btn.addEventListener('click', () => {
      const md = page.markdown || '';
      const done = () => { btn.textContent = 'copied'; setTimeout(() => { btn.textContent = 'copy markdown'; }, 1500); };
      if (navigator.clipboard) navigator.clipboard.writeText(md).then(done).catch(() => {});
    });
    rail.appendChild(btn);
  }

  function renderCitations(citations) {
    const box = el('div', { class:'cites' }, el('h3', { text:'Citations' }));
    for (const c of citations) {
      if (!c || !c.path) continue;
      const range = c.start_line ? (':' + c.start_line + (c.end_line && c.end_line !== c.start_line ? '-' + c.end_line : '')) : '';
      const label = c.path + range;
      if (c.url && /^https?:\/\//i.test(c.url)) box.appendChild(el('a', { href: c.url, target:'_blank', rel:'noopener noreferrer', text: label }));
      else box.appendChild(el('span', { class:'nolink', text: label }));
    }
    return box;
  }

  //--- search
  let searchTimer = null;
  function onSearchInput(e) {
    const q = e.target.value.trim();
    if (searchTimer) clearTimeout(searchTimer);
    if (!q) {
      state.searchResults = null;
      renderPageList();
      return;
    }
    if (!state.currentWikiId) return;
    searchTimer = setTimeout(async () => {
      try {
        const res = await api('/wikis/' + state.currentWikiId + '/search?q=' + encodeURIComponent(q));
        state.searchResults = res || [];
        renderPageList();
      } catch (err) {
        flashError('Search failed: ' + err.message);
      }
    }, 250);
  }

  //--- about modal
  function openAbout() {
    const back = el('div', { class:'modal-back', onclick: (e) => { if (e.target === back) back.remove(); } });
    const box = el('div', { class:'modal' },
      el('h2', { text:'About OpenWiki' }),
      el('p', { text:'OpenWiki turns any code repository into a browsable, source-grounded wiki: a hierarchical set of cited pages generated from the source itself, kept current from git changes.' }),
      el('p', { text:'Built as an iii worker \u2014 it composes the harness, llm-router, shell, and web workers to plan, explore, and write.' }),
      el('button', { onclick: () => back.remove(), text:'Close' }),
    );
    back.appendChild(box);
    document.body.appendChild(back);
  }

  //--- ask modal
  function openAsk() {
    if (!state.currentWikiId) return;
    const back = el('div', { class:'modal-back', onclick: (e) => { if (e.target === back) back.remove(); } });
    const answer = el('div', { class:'ask-answer' });
    const input = el('input', { class:'ask-input', placeholder:'Ask a question about this repo…', autocomplete:'off' });
    const ask = async () => {
      const q = input.value.trim();
      if (!q) return;
      answer.textContent = '';
      answer.appendChild(el('div', { class:'spinner' }));
      try {
        const res = await api('/wikis/' + state.currentWikiId + '/ask', {
          method:'POST', headers:{ 'content-type':'application/json' }, body: JSON.stringify({ q }),
        });
        answer.textContent = '';
        const md = el('div', { class:'md' });
        md.innerHTML = renderMarkdown(res.answer || '', { wikiId: state.currentWikiId });
        answer.appendChild(md);
        renderMermaidBlocks(md);
        if (res.citations && res.citations.length) answer.appendChild(renderCitations(res.citations));
      } catch (e) {
        answer.textContent = '';
        answer.appendChild(el('div', { class:'empty', text:'Ask failed: ' + e.message }));
      }
    };
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); ask(); } });
    const box = el('div', { class:'modal wide' },
      el('h2', { text:'Ask this wiki' }),
      input,
      answer,
      el('button', { onclick: () => back.remove(), text:'Close' }),
    );
    back.appendChild(box);
    document.body.appendChild(back);
    setTimeout(() => input.focus(), 0);
  }

  //--- init
  function bindUI() {
    const input = $('repo-url');
    const btn = $('gen-btn');
    input.addEventListener('input', () => { btn.disabled = !input.value.trim(); });
    $('newwiki-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const v = input.value.trim();
      if (!v) return;
      input.value = '';
      btn.disabled = true;
      generateWiki(v);
    });
    $('search').addEventListener('input', onSearchInput);
    $('ask-btn').addEventListener('click', openAsk);
    const themeBtn = $('theme-btn');
    const applyTheme = (t) => {
      document.documentElement.setAttribute('data-theme', t);
      themeBtn.textContent = t === 'dark' ? 'light' : 'dark';
      try { localStorage.setItem('ow-theme', t); } catch (_) {}
    };
    themeBtn.addEventListener('click', () =>
      applyTheme(document.documentElement.getAttribute('data-theme') === 'dark' ? 'light' : 'dark'));
    let savedTheme = 'light';
    try { savedTheme = localStorage.getItem('ow-theme') || 'light'; } catch (_) {}
    applyTheme(savedTheme);
    $('about-link').addEventListener('click', (e) => { e.preventDefault(); openAbout(); });
    window.addEventListener('hashchange', onHashChange);
  }

  function onHashChange() {
    const { wikiId, slug } = parseHash();
    if (!wikiId) return;
    if (wikiId !== state.currentWikiId) {
      selectWiki(wikiId, slug);
    } else if (slug && slug !== state.currentSlug) {
      selectPage(slug);
    }
  }

  async function boot() {
    bindUI();
    await loadWikis();
    loadModels();
    const { wikiId, slug } = parseHash();
    if (wikiId) {
      selectWiki(wikiId, slug);
    } else {
      renderMain(null);
    }
  }

  boot();
})();
</script>
</body>
</html>`;
