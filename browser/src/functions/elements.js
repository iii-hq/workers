// browser::elements — the page's visible, enabled controls in the viewport as
// an indexed table, plus the visible text, read in one evaluation. Ported from
// browser-use/jev-ultrafast's snapshot.js. Each element gets a code-owned `n`
// ref: a WeakMap keeps one id per real DOM node for the document's lifetime and
// a Map keeps the live node the ref resolves to, so a ref never names a
// different element. A new document's registry numbers from `first`, above
// every id the session handed out before, so a ref read on an earlier page
// never resolves on a later one. Worker overlays (`iii-*` ids) are never
// listed. Called with `first`; returns `next` for the session to remember.
((first) => {
  if (!document.body) return null;
  const R = (window.__iiiElements ||= { ids: new WeakMap(), nodes: new Map(), next: first });
  const identity = (e) => {
    if (!R.ids.has(e)) R.ids.set(e, R.next++);
    const id = R.ids.get(e);
    R.nodes.set(id, e);
    return id;
  };
  for (const [id, e] of R.nodes) if (!e.isConnected) R.nodes.delete(id);
  const cap = (s, n) => {
    s = String(s ?? '').replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  };
  const hidden = (e) =>
    !!e.closest('[aria-hidden="true"],[inert],[id^="iii-"]') ||
    !e.checkVisibility({ checkVisibilityCSS: true });
  const name = (e, seen = new Set()) => {
    if (!e || seen.has(e)) return '';
    seen.add(e);
    const referenced = (e.getAttribute('aria-labelledby') || '')
      .split(/\s+/)
      .map((id) => name(document.getElementById(id), seen))
      .filter(Boolean)
      .join(' ');
    return (
      referenced ||
      e.getAttribute('aria-label') ||
      [...(e.labels || [])].map((l) => name(l, seen)).filter(Boolean).join(' ') ||
      (['button', 'submit', 'reset'].includes(e.type) ? e.value : '') ||
      e.getAttribute('alt') ||
      (e.tagName === 'INPUT' || e.tagName === 'SELECT' || e.tagName === 'TEXTAREA'
        ? ''
        : [...e.childNodes]
            .map((n) =>
              n.nodeType === 3
                ? n.textContent
                : n.nodeType === 1 && n.getAttribute('aria-hidden') !== 'true'
                  ? name(n, seen)
                  : '',
            )
            .join(' ')
            .trim()) ||
      e.getAttribute('title') ||
      e.getAttribute('placeholder') ||
      ''
    );
  };
  const roles = ['button', 'link', 'checkbox', 'radio', 'switch', 'tab', 'menuitem',
    'menuitemcheckbox', 'menuitemradio', 'option', 'gridcell', 'combobox', 'textbox',
    'searchbox', 'spinbutton', 'slider'];
  const selector =
    'a[href],button,input,textarea,select,summary,[contenteditable]:not([contenteditable="false"]),' +
    roles.map((role) => '[role="' + role + '"]').join(',');
  const role = (e) => {
    const explicit = e.getAttribute('role');
    if (roles.includes(explicit)) return explicit;
    if (e.tagName === 'BUTTON' || e.tagName === 'SUMMARY') return 'button';
    if (e.tagName === 'A') return 'link';
    if (e.tagName === 'SELECT') return 'combobox';
    if (e.tagName === 'TEXTAREA' || e.isContentEditable) return 'textbox';
    if (e.tagName === 'INPUT') {
      if (['checkbox', 'radio'].includes(e.type)) return e.type;
      if (['button', 'submit', 'reset', 'image'].includes(e.type)) return 'button';
      if (e.type === 'search') return 'searchbox';
      if (e.type === 'number') return 'spinbutton';
      if (['text', 'email', 'url', 'tel', 'password'].includes(e.type)) return 'textbox';
    }
    return null;
  };
  const elements = [];
  let omitted = 0;
  for (const e of document.querySelectorAll(selector)) {
    if (e.tagName === 'INPUT' && ['file', 'hidden'].includes(e.type)) continue;
    if (e.matches(':disabled') || e.closest('[aria-disabled="true"]') || hidden(e)) continue;
    const rname = role(e);
    const r = e.getBoundingClientRect();
    const x = r.x + r.width / 2;
    const y = r.y + r.height / 2;
    if (!rname || r.width <= 0 || r.height <= 0 || x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) continue;
    if (rname === 'gridcell' && e.querySelector('button,[role="button"]')) continue;
    if (elements.length >= 250) {
      omitted++;
      continue;
    }
    const el = { ref: 'n' + identity(e), role: rname, label: cap(name(e) || rname, 120), operations: [] };
    for (const key of ['checked', 'selected', 'expanded']) {
      const value = e.getAttribute('aria-' + key);
      if (value !== null) el[key] = value;
    }
    if (['checkbox', 'radio'].includes(e.type)) el.checked = String(e.checked);
    // What the field is, beyond its label: lets a caller's `inputs` key such
    // as `password` or `email` find the field whatever language labels it.
    if (e.tagName === 'INPUT' || e.tagName === 'TEXTAREA') {
      if (e.tagName === 'INPUT' && e.type && e.type !== 'text') el.input_type = e.type;
      const fieldName = e.getAttribute('name') || e.id;
      if (fieldName) el.name = cap(fieldName, 60);
      const auto = e.getAttribute('autocomplete');
      if (auto && auto !== 'on' && auto !== 'off') el.autocomplete = cap(auto, 60);
    }
    if (e.tagName === 'SELECT') {
      el.operations.push('select');
      el.value = cap([...e.selectedOptions].map((o) => o.label).join(', '), 120);
      const options = [...e.options].filter((o) => !o.disabled && !o.closest('optgroup[disabled]'));
      el.options = options.slice(0, 50).map((o) => cap(o.label, 80));
      if (options.length > 50) el.more_options = options.length - 50;
    } else {
      const editable =
        !e.readOnly &&
        e.getAttribute('aria-readonly') !== 'true' &&
        (['textbox', 'searchbox', 'spinbutton'].includes(rname) ||
          (rname === 'combobox' && ['INPUT', 'TEXTAREA'].includes(e.tagName)));
      if (editable) el.operations.push('type');
      el.operations.push('click');
      if (e.type === 'password') el.value = e.value ? '(set)' : '';
      else if (editable || rname === 'combobox')
        el.value = cap('value' in e ? e.value : e.innerText, 120);
      if (el.value === '') delete el.value;
    }
    elements.push(el);
  }
  const words = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  let node;
  let length = 0;
  while ((node = walker.nextNode()) && length < 6000) {
    const value = node.textContent.replace(/\s+/g, ' ').trim();
    const parent = node.parentElement;
    if (!value || !parent || parent.closest('script,style,noscript,template') || hidden(parent)) continue;
    range.selectNodeContents(node);
    const r = range.getBoundingClientRect();
    if (r.width > 0 && r.height > 0 && r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth) {
      words.push(value);
      length += value.length + 1;
    }
  }
  const height = document.documentElement.scrollHeight;
  return {
    url: location.href,
    title: document.title,
    // Still loading: the document itself, or a region the page marks busy.
    busy: document.readyState !== 'complete' || !!document.querySelector('[aria-busy="true"]'),
    text: words.join('\n').slice(0, 6000),
    can_scroll_up: scrollY > 0,
    can_scroll_down: scrollY + innerHeight < height - 2,
    elements,
    omitted,
    next: R.next,
  };
})
