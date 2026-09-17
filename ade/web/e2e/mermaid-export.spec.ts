/// <reference lib="dom" />

import { readFileSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import ts from 'typescript'

// Exercise the production export code with real SVG layout, not jsdom's
// mocked getBBox. No engine, chat data, network or development server needed.
const moduleSource = ts.transpileModule(
  readFileSync(new URL('../src/lib/mermaid.ts', import.meta.url), 'utf8'),
  {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ESNext,
    },
  },
).outputText
// Optional local override when Playwright's bundled Chromium is not installed.
test.use({ channel: process.env.CONSOLE_E2E_BROWSER_CHANNEL, video: 'off' })

for (const theme of ['light', 'dark']) {
  test(`SVG export contains the entire drawing in ${theme} theme`, async ({
    page,
  }) => {
    await page.setContent(`
      <style>
        /* Host application CSS must not contaminate standalone SVG metrics. */
        svg text { font-size: 80px !important; }
      </style>
      <div id="result"></div>
    `)
    const checkExport = async ({
      moduleSource,
      theme,
    }: {
      moduleSource: string
      theme: string
    }) => {
      const moduleUrl = URL.createObjectURL(
        new Blob([moduleSource], { type: 'text/javascript' }),
      )
      const { normalizeMermaidSvg } = await import(/* @vite-ignore */ moduleUrl)
      URL.revokeObjectURL(moduleUrl)
      // Reproduce the screenshot's failure: drawing outside an 868 x 218
      // viewport, including translated bottom and right-hand nodes.
      const source = `<svg xmlns="http://www.w3.org/2000/svg" width="868" height="218" viewBox="0 0 868 218">
        <style>text{font:16px sans-serif;fill:${theme === 'dark' ? '#eee' : '#111'}}</style>
        <g transform="translate(90 65)">
          <rect x="350" y="0" width="250" height="65" fill="red"/>
          <text x="365" y="35">TriggerRegistry</text>
          <g transform="translate(0 170)">
            <rect width="260" height="75" fill="red"/>
            <text x="10" y="35">Trigger</text>
            <rect x="330" width="260" height="75" fill="red"/>
            <text x="340" y="35">TriggerType + registrator</text>
            <rect x="660" width="260" height="75" fill="red"/>
            <text x="670" y="35">BindingLifecycle</text>
          </g>
        </g>
      </svg>`
      const baseline = document.body.childElementCount
      const normalized = normalizeMermaidSvg(source)
      const cleanedUp = document.body.childElementCount === baseline
      const url = URL.createObjectURL(
        new Blob([normalized], { type: 'image/svg+xml' }),
      )
      const image = new Image()
      image.src = url
      await image.decode()
      const canvas = document.createElement('canvas')
      canvas.width = image.naturalWidth
      canvas.height = image.naturalHeight
      const ctx = canvas.getContext('2d')
      if (!ctx) throw new Error('Canvas unavailable')
      ctx.drawImage(image, 0, 0)
      const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data
      let paintedRight = 0
      let paintedBottom = 0
      for (let y = 0; y < canvas.height; y++) {
        for (let x = 0; x < canvas.width; x++) {
          if (pixels[(y * canvas.width + x) * 4 + 3] > 0) {
            paintedRight = Math.max(paintedRight, x)
            paintedBottom = Math.max(paintedBottom, y)
          }
        }
      }
      URL.revokeObjectURL(url)
      const parsed = new DOMParser().parseFromString(
        normalized,
        'image/svg+xml',
      )
      return {
        cleanedUp,
        viewBox: parsed.documentElement.getAttribute('viewBox'),
        width: image.naturalWidth,
        height: image.naturalHeight,
        paintedRight,
        paintedBottom,
        labels: [...parsed.querySelectorAll('text')].map(
          (el) => el.textContent,
        ),
      }
    }
    const result = await page.evaluate(checkExport, { moduleSource, theme })
    expect(result.cleanedUp).toBe(true)
    expect(result.viewBox).toBe('-16 -16 1042 342')
    expect(result.width).toBe(1042)
    expect(result.height).toBe(342)
    expect(result.paintedRight).toBe(1025)
    expect(result.paintedBottom).toBe(325)
    expect(result.width - result.paintedRight).toBeGreaterThan(15)
    expect(result.height - result.paintedBottom).toBeGreaterThan(15)
    expect(result.labels).toEqual([
      'TriggerRegistry',
      'Trigger',
      'TriggerType + registrator',
      'BindingLifecycle',
    ])
  })
}
