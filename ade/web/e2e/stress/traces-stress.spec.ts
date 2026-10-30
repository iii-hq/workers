/**
 * Traces stress test — `pnpm test:stress`.
 *
 * Loads the traces page over generated traces of growing size and measures
 * what a reviewer actually sees on a busy engine: how many DOM nodes the
 * views keep alive, how much main-thread time a live stream burns, how the
 * heap grows, and how long the longest frame stalls. Every figure is read
 * off the Chrome DevTools protocol, written to a JSON report under
 * `target/console-stress/`, and compared with a budget.
 *
 * Phases per size (`STRESS_SIZES`, default 500,2000,5000):
 *   load      open the page, wait for the timeline bars, GC, read nodes/heap
 *   stream    advance the live stream through 10 ticks, read CPU + long tasks
 *   hover     sweep the pointer across the timeline (the hover card path)
 *   waterfall switch views, scroll the virtualised list, read nodes
 *
 * Budgets scale with the trace: the DOM must stay bounded by the viewport,
 * not by the span count. A laptop is a noisy bench, so each size is
 * measured `STRESS_REPEAT` times (default 2) and the quietest sample of
 * every figure is what the budget sees — interference can only make a run
 * look worse, never better. `STRESS_RELAX=2` doubles every budget for a
 * slow machine; `STRESS_BASELINE=1` records without asserting.
 */

import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { type CDPSession, expect, type Page, test } from '@playwright/test'

declare global {
  interface Window {
    __stress?: {
      ready: Promise<void>
      spanCount: number
      advance: (fraction: number) => Promise<void>
      calls: Record<string, number>
    }
  }
}

const SIZES = (process.env.STRESS_SIZES ?? '500,2000,5000')
  .split(',')
  .map((n) => Number.parseInt(n, 10))
  .filter((n) => Number.isFinite(n) && n > 0)
const RELAX = Number.parseFloat(process.env.STRESS_RELAX ?? '1') || 1
const REPEAT =
  Math.max(1, Number.parseInt(process.env.STRESS_REPEAT ?? '2', 10)) || 2
const BASELINE = process.env.STRESS_BASELINE === '1'
const STREAM_TICKS = 10
const HOVER_STEPS = 40
const REPORT_DIR = path.resolve(
  process.env.CONSOLE_STRESS_ARTIFACTS_DIR ??
    path.join(import.meta.dirname, '../../../../target/console-stress'),
)

/**
 * The budgets. Each is a ceiling on one phase for one size; they were set
 * from a measured run on an M-series laptop (2026-10-23: 500 / 2000 / 5000
 * spans → stream 152 / 250 / 265 ms per tick, hover sweep 582 / 654 / 790
 * ms, longest task 71 / 87 / 119 ms, heap 52 / 62 / 76 MB) with roughly
 * 1.4× headroom, so a change that doubles the work fails here before a
 * reviewer sees it. `STRESS_RELAX` scales them for a slower machine.
 */
function budgets(spans: number) {
  // Timeline bars that can intersect the stage at once: the viewport is
  // ~25 packed lines at 60dvh, and a dense line holds dozens of bars.
  const viewportBars = 1_500
  return {
    /** bars the timeline keeps in the DOM: bounded by the viewport, not the trace */
    timelineBars: Math.min(spans, viewportBars),
    /** total elements inside the traces section after the detail opens */
    loadNodes: 2_500 + Math.min(spans, viewportBars) * 6,
    /** main-thread ms per streamed tick (script + style + layout) */
    streamTaskMsPerTick: 220 + spans * 0.03,
    /** longest single task during the stream, ms */
    streamLongestTaskMs: 110 + spans * 0.02,
    /** main-thread ms for the whole hover sweep (includes the live strip's
     *  animation frames over the same window) */
    hoverTaskMs: 800 + spans * 0.06,
    /** JS heap after the stream and a GC, MB */
    heapMb: 70 + spans * 0.004,
    /** waterfall rows in the DOM (virtualised) */
    waterfallRows: 80,
  }
}

interface Metrics {
  TaskDuration: number
  ScriptDuration: number
  LayoutDuration: number
  RecalcStyleDuration: number
  JSHeapUsedSize: number
  Nodes: number
  LayoutCount: number
}

async function readMetrics(cdp: CDPSession): Promise<Metrics> {
  const { metrics } = await cdp.send('Performance.getMetrics')
  const out: Record<string, number> = {}
  for (const m of metrics) out[m.name] = m.value
  return out as unknown as Metrics
}

async function gc(cdp: CDPSession) {
  await cdp.send('HeapProfiler.collectGarbage')
}

async function longTasks(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as { __longTasks: number[] }
    const list = w.__longTasks ?? []
    w.__longTasks = []
    return {
      count: list.length,
      longestMs: list.length ? Math.max(...list) : 0,
      totalMs: list.reduce((a, b) => a + b, 0),
    }
  })
}

async function domFigures(page: Page) {
  return page.evaluate(() => {
    const section = document.querySelector('section[aria-label="traces"]')
    return {
      sectionNodes: section ? section.querySelectorAll('*').length : 0,
      documentNodes: document.querySelectorAll('*').length,
      timelineBars: document.querySelectorAll('[data-trace-span-id]').length,
      waterfallRows: document.querySelectorAll('[data-index]').length,
    }
  })
}

const ms = (n: number) => Math.round(n * 1000) / 1000
const mb = (bytes: number) => Math.round((bytes / 1024 / 1024) * 10) / 10

test.describe('traces stress', () => {
  const report: Record<string, unknown>[] = []

  test.afterAll(() => {
    mkdirSync(REPORT_DIR, { recursive: true })
    const file = path.join(REPORT_DIR, 'report.json')
    writeFileSync(
      file,
      JSON.stringify(
        { at: new Date().toISOString(), relax: RELAX, runs: report },
        null,
        2,
      ),
    )
    // One table a reviewer can read off the terminal.
    const rows = report.map((r) => ({
      spans: r.spans,
      'load ms': r.loadMs,
      bars: (r.load as { timelineBars: number }).timelineBars,
      'nodes (section)': (r.load as { sectionNodes: number }).sectionNodes,
      'stream ms/tick': (r.stream as { taskMsPerTick: number }).taskMsPerTick,
      'longest task ms': (r.stream as { longestTaskMs: number }).longestTaskMs,
      'hover ms': (r.hover as { taskMs: number }).taskMs,
      'heap MB': (r.stream as { heapMb: number }).heapMb,
      'wf rows': (r.waterfall as { rows: number }).rows,
    }))
    console.table(rows)
    console.log(`stress report → ${file}`)
  })

  for (const spans of SIZES) {
    test(`renders, streams and scrolls a ${spans}-span trace within budget`, async ({
      page,
    }) => {
      const budget = budgets(spans)
      const samples: Measurement[] = []
      for (let attempt = 0; attempt < REPEAT; attempt++) {
        samples.push(await measure(page, spans))
      }
      const best = quietest(samples)
      const run = { spans, ...best, attempts: samples.length, budget }
      report.push(run)
      console.log(JSON.stringify(run))

      if (BASELINE) return
      const soft = (actual: number, limit: number, what: string) =>
        expect
          .soft(
            actual,
            `${what}: ${actual} > budget ${Math.round(limit * RELAX)}`,
          )
          .toBeLessThanOrEqual(limit * RELAX)
      soft(
        best.load.timelineBars,
        budget.timelineBars,
        'timeline bars in the DOM',
      )
      soft(
        best.load.sectionNodes,
        budget.loadNodes,
        'DOM nodes in the traces section',
      )
      soft(
        best.stream.taskMsPerTick,
        budget.streamTaskMsPerTick,
        'main-thread ms per streamed tick',
      )
      soft(
        best.stream.longestTaskMs,
        budget.streamLongestTaskMs,
        'longest task during the stream',
      )
      soft(
        best.hover.taskMs,
        budget.hoverTaskMs,
        'main-thread ms for the hover sweep',
      )
      soft(best.stream.heapMb, budget.heapMb, 'JS heap after the stream (MB)')
      soft(
        best.waterfall.rows,
        budget.waterfallRows,
        'waterfall rows in the DOM',
      )
    })
  }
})

type Figures = Record<string, number>

interface Measurement {
  loadMs: number
  load: Figures
  stream: Figures
  hover: Figures
  waterfall: Figures
  engineCalls: Record<string, number>
}

/** Field-wise minimum across samples: the least-disturbed reading of each. */
function quietest(samples: Measurement[]): Measurement {
  const minOf = (key: 'load' | 'stream' | 'hover' | 'waterfall'): Figures => {
    const out: Figures = {}
    for (const sample of samples) {
      for (const [k, v] of Object.entries(sample[key])) {
        out[k] = k in out ? Math.min(out[k], v) : v
      }
    }
    return out
  }
  return {
    loadMs: Math.min(...samples.map((s) => s.loadMs)),
    load: minOf('load'),
    stream: minOf('stream'),
    hover: minOf('hover'),
    waterfall: minOf('waterfall'),
    engineCalls: samples[samples.length - 1].engineCalls,
  }
}

/** One full pass over the four phases for one trace size. */
async function measure(page: Page, spans: number): Promise<Measurement> {
  await page.addInitScript(() => {
    const w = window as unknown as { __longTasks: number[] }
    w.__longTasks = []
    try {
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) w.__longTasks.push(e.duration)
      }).observe({ type: 'longtask', buffered: true })
    } catch {
      /* no longtask support */
    }
  })
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('Performance.enable')
  await cdp.send('HeapProfiler.enable')

  // ── load ──
  const t0 = Date.now()
  await page.goto(`/stress.html?spans=${spans}&stream=1&theme=dark`)
  await page.waitForFunction(() => !!window.__stress)
  await expect(page.locator('[data-trace-span-id]').first()).toBeVisible({
    timeout: 120_000,
  })
  // the seed sweep repaints at most every 600ms; let it settle
  await page.waitForFunction(
    () =>
      !document.querySelector('[role="status"][aria-busy="true"]') &&
      document.querySelectorAll('[data-trace-span-id]').length > 0,
  )
  await page.waitForTimeout(800)
  const loadMs = Date.now() - t0
  await gc(cdp)
  const load = { ...(await domFigures(page)), ...(await longTasks(page)) }
  const loadMetrics = await readMetrics(cdp)

  // ── stream ──
  const before = await readMetrics(cdp)
  await longTasks(page)
  const streamStart = Date.now()
  for (let i = 1; i <= STREAM_TICKS; i++) {
    const fraction = 0.3 + (0.7 * i) / STREAM_TICKS
    await page.evaluate((f) => window.__stress?.advance(f), fraction)
    await page.waitForTimeout(250)
  }
  await page.waitForTimeout(500)
  const after = await readMetrics(cdp)
  const streamLong = await longTasks(page)
  await gc(cdp)
  const afterGc = await readMetrics(cdp)
  const stream = {
    wallMs: Date.now() - streamStart,
    taskMs: ms(after.TaskDuration - before.TaskDuration) * 1000,
    taskMsPerTick: ms(
      ((after.TaskDuration - before.TaskDuration) * 1000) / STREAM_TICKS,
    ),
    scriptMs: ms((after.ScriptDuration - before.ScriptDuration) * 1000),
    layoutMs: ms((after.LayoutDuration - before.LayoutDuration) * 1000),
    styleMs: ms(
      (after.RecalcStyleDuration - before.RecalcStyleDuration) * 1000,
    ),
    layouts: after.LayoutCount - before.LayoutCount,
    longTasks: streamLong.count,
    longestTaskMs: ms(streamLong.longestMs),
    heapMb: mb(afterGc.JSHeapUsedSize),
    ...(await domFigures(page)),
  }

  // ── hover ──
  const stage = page.locator('[data-trace-span-id]').first()
  const box = (await page
    .locator('section[aria-label="traces"]')
    .boundingBox()) ?? { x: 0, y: 0, width: 1440, height: 900 }
  const barBox = (await stage.boundingBox()) ?? box
  const hoverBefore = await readMetrics(cdp)
  await longTasks(page)
  for (let i = 0; i < HOVER_STEPS; i++) {
    const x = box.x + 40 + ((box.width - 80) * i) / HOVER_STEPS
    const y = barBox.y + 8 + (i % 6) * 22
    await page.mouse.move(x, y)
    await page.waitForTimeout(16)
  }
  await page.waitForTimeout(200)
  const hoverAfter = await readMetrics(cdp)
  const hoverLong = await longTasks(page)
  const hover = {
    taskMs: ms((hoverAfter.TaskDuration - hoverBefore.TaskDuration) * 1000),
    longestTaskMs: ms(hoverLong.longestMs),
  }

  // ── waterfall ──
  const wfBefore = await readMetrics(cdp)
  await page.getByRole('tab', { name: 'Waterfall' }).click()
  await expect(page.locator('[data-index]').first()).toBeVisible()
  const list = page
    .locator('[data-index]')
    .first()
    .locator('xpath=ancestor::div[contains(@class,"overflow-y-auto")][1]')
  for (let i = 0; i < 10; i++) {
    await list.evaluate((el, step) => {
      el.scrollTop += step
    }, 320)
    await page.waitForTimeout(60)
  }
  await page.waitForTimeout(300)
  const wfAfter = await readMetrics(cdp)
  const waterfall = {
    rows: (await domFigures(page)).waterfallRows,
    taskMs: ms((wfAfter.TaskDuration - wfBefore.TaskDuration) * 1000),
    ...(await longTasks(page)),
  }

  await cdp.detach()
  return {
    loadMs,
    load: { ...load, heapMb: mb(loadMetrics.JSHeapUsedSize) },
    stream,
    hover,
    waterfall,
    engineCalls: await page.evaluate(() => window.__stress?.calls ?? {}),
  }
}
