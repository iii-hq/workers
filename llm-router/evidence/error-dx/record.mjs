import { mkdir, readdir, rename, rm } from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const directory = path.dirname(fileURLToPath(import.meta.url))
const require = createRequire(
  path.join(directory, '../../../console/web/package.json'),
)
const { chromium } = require('@playwright/test')
const temporaryDirectory = path.join(directory, '.recording')
const videoPath = path.join(directory, 'error-dx-demo.webm')
const posterPath = path.join(directory, 'error-dx-poster.png')
const slideDurations = [3800, 5000, 4800, 5000, 5000, 5200]

await rm(temporaryDirectory, { recursive: true, force: true })
await mkdir(temporaryDirectory, { recursive: true })

const browser = await chromium.launch({ headless: true })
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  colorScheme: 'dark',
  recordVideo: {
    dir: temporaryDirectory,
    size: { width: 1440, height: 900 },
  },
})
const page = await context.newPage()
await page.goto(pathToFileURL(path.join(directory, 'presentation.html')).href)
await page.waitForFunction(() => document.fonts.status === 'loaded')
await page.screenshot({ path: posterPath })

for (const [index, duration] of slideDurations.entries()) {
  await page.evaluate((slide) => window.showSlide(slide), index)
  await page.waitForTimeout(duration)
}

const video = page.video()
await page.close()
await context.close()
await browser.close()

if (!video) {
  throw new Error('Playwright did not create a video')
}
const generatedPath = await video.path()
await rm(videoPath, { force: true })
await rename(generatedPath, videoPath)
await rm(temporaryDirectory, { recursive: true, force: true })

const [generated] = await readdir(directory).then((files) =>
  files.filter((file) => file === path.basename(videoPath)),
)
console.log(`Recorded ${generated} and ${path.basename(posterPath)}`)
