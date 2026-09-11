import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import type { IIIClient } from 'iii-sdk'
import { SLIDE_HEIGHT, SLIDE_WIDTH } from './render-html.js'

export const CAPTURE_SCALE = 2
const SESSION_TTL_MS = 5 * 60_000
const CALL_TIMEOUT_MS = 60_000
const VIEWPORT_ATTEMPTS = 4

export interface SlideImage {
  index: number
  content_type: string
  width: number
  height: number
  data_base64: string
}

export interface BlockMeasure {
  block_id: string
  type: string
  x: number
  y: number
  width: number
  height: number
  overflow: boolean
}

export interface SlideMeasure {
  index: number
  fit: number
  overflow: boolean
  overflow_px: number
  minimum_font_size: number
  empty_space_ratio: number
  clipped_labels: string[]
  collisions: { a: string; b: string }[]
  blocks: BlockMeasure[]
}

type ContentBlock = { type: string; mime?: string; data?: string }
type Screenshot = { content: ContentBlock[]; details: { width: number; height: number } }
type Execute = { ok: boolean; result?: unknown; error?: string }

export class CaptureUnavailable extends Error {
  constructor(message: string) {
    super(`CAPTURE_UNAVAILABLE: ${message}`)
  }
}

function browserError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error)
  if (/not.?found|no such function|function_not_found|unknown function/i.test(message))
    throw new CaptureUnavailable('the browser worker is not registered; install it with compose::add worker=browser')
  throw new Error(`CAPTURE_FAILED: ${message}`)
}

export class Page {
  constructor(
    private readonly iii: IIIClient,
    readonly sessionId: string,
    private readonly url: string,
  ) {}

  private call<T>(function_id: string, payload: Record<string, unknown>): Promise<T> {
    return this.iii
      .trigger<Record<string, unknown>, T>({ function_id, payload, timeoutMs: CALL_TIMEOUT_MS })
      .catch(browserError)
  }

  async open(scale: number): Promise<void> {
    this.scale = scale
    await this.ensureViewport()
    await this.call('browser::navigate', { session_id: this.sessionId, url: `${this.url}#1`, timeout_ms: 30_000 })
    await this.execute(SETTLE_SCRIPT)
    await this.ensureViewport()
  }

  private scale = 1

  async ensureViewport(): Promise<void> {
    for (let attempt = 0; attempt < VIEWPORT_ATTEMPTS; attempt += 1) {
      await this.call('browser::resize', {
        session_id: this.sessionId,
        width: SLIDE_WIDTH,
        height: SLIDE_HEIGHT,
        device_scale_factor: this.scale,
      })
      const size = await this.execute<{ w: number; h: number; dpr: number }>(VIEWPORT_SCRIPT)
      if (size.w === SLIDE_WIDTH && size.h === SLIDE_HEIGHT && Math.abs(size.dpr - this.scale) < 0.01) return
    }
    throw new Error(`CAPTURE_FAILED: the browser viewport did not settle at ${SLIDE_WIDTH}x${SLIDE_HEIGHT}`)
  }

  async execute<T>(code: string): Promise<T> {
    const out = await this.call<Execute>('browser::execute', {
      session_id: this.sessionId,
      code,
      timeout_ms: 30_000,
    })
    if (!out.ok) throw new Error(`CAPTURE_FAILED: page script failed: ${out.error ?? 'unknown error'}`)
    return out.result as T
  }

  async goto(index: number): Promise<void> {
    await this.ensureViewport()
    await this.execute(`location.hash = '#${index + 1}'; await sleep(40); return true`)
  }

  async screenshot(index: number): Promise<SlideImage> {
    const shot = await this.call<Screenshot>('browser::screenshot', { session_id: this.sessionId })
    const image = shot.content.find((block) => block.type === 'image' && block.data)
    if (!image?.data) throw new Error('CAPTURE_FAILED: the browser returned no image')
    return {
      index,
      content_type: image.mime ?? 'image/jpeg',
      width: Math.round(shot.details.width * this.scale),
      height: Math.round(shot.details.height * this.scale),
      data_base64: image.data,
    }
  }

  async close(): Promise<void> {
    await this.iii
      .trigger({ function_id: 'browser::sessions::stop', payload: { session_id: this.sessionId }, timeoutMs: 10_000 })
      .catch(() => undefined)
  }
}

export async function withPage<T>(
  iii: IIIClient,
  html: string,
  fn: (page: Page) => Promise<T>,
  scale = CAPTURE_SCALE,
): Promise<T> {
  const dir = await mkdtemp(join(tmpdir(), 'slides-capture-'))
  const file = join(dir, 'deck.html')
  await writeFile(file, html, 'utf8')
  let page: Page | null = null
  try {
    const started = await iii
      .trigger<Record<string, unknown>, { session_id: string }>({
        function_id: 'browser::sessions::start',
        payload: { incognito: true, ttl_ms: SESSION_TTL_MS },
        timeoutMs: CALL_TIMEOUT_MS,
      })
      .catch(browserError)
    page = new Page(iii, started.session_id, pathToFileURL(file).href)
    await page.open(scale)
    return await fn(page)
  } finally {
    await page?.close()
    await rm(dir, { recursive: true, force: true }).catch(() => undefined)
  }
}

export async function captureSlides(
  iii: IIIClient,
  html: string,
  indexes: number[],
  scale = CAPTURE_SCALE,
): Promise<SlideImage[]> {
  return withPage(
    iii,
    html,
    async (page) => {
      const images: SlideImage[] = []
      for (const index of indexes) {
        await page.goto(index)
        images.push(await page.screenshot(index))
      }
      return images
    },
    scale,
  )
}

export async function captureOverview(iii: IIIClient, html: string, scale = 1): Promise<SlideImage> {
  return withPage(
    iii,
    html,
    async (page) => {
      await page.execute(
        "document.body.classList.add('overview'); document.querySelector('.stage').style.position = 'static'; document.querySelector('.stage').style.overflow = 'visible'; await sleep(60); return true",
      )
      const shot = await iii
        .trigger<Record<string, unknown>, Screenshot>({
          function_id: 'browser::screenshot',
          payload: { session_id: page.sessionId, full_page: true },
          timeoutMs: CALL_TIMEOUT_MS,
        })
        .catch(browserError)
      const image = shot.content.find((block) => block.type === 'image' && block.data)
      if (!image?.data) throw new Error('CAPTURE_FAILED: the browser returned no image')
      return {
        index: -1,
        content_type: image.mime ?? 'image/jpeg',
        width: shot.details.width,
        height: shot.details.height,
        data_base64: image.data,
      }
    },
    scale,
  )
}

export async function measureSlides(iii: IIIClient, html: string): Promise<SlideMeasure[]> {
  return withPage(iii, html, (page) => page.execute<SlideMeasure[]>(MEASURE_SCRIPT), 1)
}

const VIEWPORT_SCRIPT = `
await sleep(30);
return { w: window.innerWidth, h: window.innerHeight, dpr: window.devicePixelRatio };
`

const SETTLE_SCRIPT = `
await waitFor('.slide.active', { timeout: 15000 });
if (document.fonts && document.fonts.ready) await Promise.race([document.fonts.ready, sleep(8000)]);
await sleep(120);
return document.querySelectorAll('.slide').length;
`

const MEASURE_SCRIPT = String.raw`
const slides = Array.from(document.querySelectorAll('.slide'));
const out = [];
const CELL = 20;
const rectOf = (el) => el.getBoundingClientRect();
const frame = document.querySelector('.frame');
const origin = rectOf(frame);
const k = origin.width / frame.offsetWidth || 1;
const probe = document.createElement('span');
probe.style.fontFamily = 'var(--font-mono)';
document.body.appendChild(probe);
const mono = getComputedStyle(probe).fontFamily;
probe.remove();
const local = (r) => ({ x: (r.left - origin.left) / k, y: (r.top - origin.top) / k, width: r.width / k, height: r.height / k });
const textElements = (root) => Array.from(root.querySelectorAll('*')).filter((el) =>
  Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim()) && rectOf(el).width > 0);
for (const slide of slides) {
  const index = Number(slide.getAttribute('data-index'));
  location.hash = '#' + (index + 1);
  await sleep(30);
  const body = slide.querySelector('.body') || slide;
  const bodyRect = local(rectOf(body));
  const root = slide.querySelector(':scope > .body > .stack, :scope > .body > .split, :scope > .stack, :scope > .split');
  const fit = root ? Number(getComputedStyle(root).getPropertyValue('--fit') || 1) || 1 : 1;
  const overflowPx = root ? Math.max(0, root.scrollHeight - root.clientHeight) : 0;
  let minFont = Infinity;
  for (const el of textElements(slide)) {
    if (el.closest('svg, .notes, .slide-footer, .kicker, .meta, .chapter')) continue;
    if (!el.closest('[data-block-id], .slide-title, .slide-subtitle, .deck-title')) continue;
    const style = getComputedStyle(el);
    if (style.textTransform === 'uppercase' || style.fontFamily === mono) continue;
    const size = parseFloat(style.fontSize) * (root && root.contains(el) ? fit : 1);
    if (size > 0 && size < minFont) minFont = size;
  }
  const blocks = Array.from(slide.querySelectorAll('[data-block-id]')).map((el) => {
    const r = local(rectOf(el));
    const bottom = r.y + r.height;
    const overflow = bottom > bodyRect.y + bodyRect.height + 2 || r.x + r.width > bodyRect.x + bodyRect.width + 2
      || el.scrollHeight > el.clientHeight + 2;
    return { block_id: el.getAttribute('data-block-id'), type: (el.className.match(/\bblock-([a-z-]+)/) || [])[1] || el.tagName.toLowerCase(), x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height), overflow };
  });
  const collisions = [];
  for (let i = 0; i < blocks.length; i++) for (let j = i + 1; j < blocks.length; j++) {
    const a = blocks[i], b = blocks[j];
    const ox = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
    const oy = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
    if (ox > 6 && oy > 6) collisions.push({ a: a.block_id, b: b.block_id });
  }
  const clipped = [];
  for (const svg of slide.querySelectorAll('svg.diagram, svg.chart')) {
    const box = rectOf(svg);
    for (const label of svg.querySelectorAll('text')) {
      const r = rectOf(label);
      if (r.width && (r.left < box.left - 1 || r.right > box.right + 1 || r.top < box.top - 1 || r.bottom > box.bottom + 1))
        clipped.push((label.textContent || '').trim().slice(0, 40));
    }
  }
  for (const el of textElements(slide)) {
    if (el.closest('svg, .notes, .slide-footer')) continue;
    const style = getComputedStyle(el);
    if (style.overflow !== 'visible' && el.scrollWidth > el.clientWidth + 2) clipped.push((el.textContent || '').trim().slice(0, 40));
  }
  const cols = Math.ceil(bodyRect.width / CELL), rows = Math.ceil(bodyRect.height / CELL);
  const grid = new Uint8Array(Math.max(1, cols * rows));
  const paint = (r) => {
    const x0 = Math.max(0, Math.floor((r.x - bodyRect.x) / CELL)), x1 = Math.min(cols - 1, Math.floor((r.x + r.width - bodyRect.x) / CELL));
    const y0 = Math.max(0, Math.floor((r.y - bodyRect.y) / CELL)), y1 = Math.min(rows - 1, Math.floor((r.y + r.height - bodyRect.y) / CELL));
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) grid[y * cols + x] = 1;
  };
  for (const el of slide.querySelectorAll('[data-block-id], .slide-title, .slide-subtitle, .kicker, .deck-title')) paint(local(rectOf(el)));
  let filled = 0;
  for (let k = 0; k < grid.length; k++) filled += grid[k];
  out.push({
    index,
    fit,
    overflow: overflowPx > 2 || blocks.some((b) => b.overflow),
    overflow_px: Math.round(overflowPx),
    minimum_font_size: Number.isFinite(minFont) ? Math.round(minFont * 10) / 10 : 0,
    empty_space_ratio: Math.round((1 - filled / grid.length) * 100) / 100,
    clipped_labels: Array.from(new Set(clipped)).slice(0, 12),
    collisions,
    blocks,
  });
}
return out;
`
