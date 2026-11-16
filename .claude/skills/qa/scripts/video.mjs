// One QA video from a run's clips: an intro card, a card before each group,
// every clip in order (scaled and padded to 1440×900, phone clips centred),
// and an outro card with the findings count. Run from ade/web (for
// @playwright/test), from a copy under ade/web/node_modules/.cache/:
//
//   cp .claude/skills/qa/scripts/video.mjs ade/web/node_modules/.cache/qa-video.mjs
//   node ade/web/node_modules/.cache/qa-video.mjs <run-dir> [groupDir=Title ...]
//
// Needs ffmpeg. Writes <run-dir>/qa-video.mp4.
import { chromium } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const run = process.argv[2] && resolve(process.argv[2])
if (!run) throw new Error('usage: video.mjs <run-dir> [groupDir=Title ...]')
const groups = (process.argv.slice(3).length ? process.argv.slice(3) : readdirSync(run).filter((d) => existsSync(join(run, d, 'videos'))).map((d) => `${d}=${d}`))
  .map((g) => { const [dir, ...t] = g.split('='); return { dir, title: t.join('=') || dir } })
const work = join(run, '.video-build')
mkdirSync(work, { recursive: true })
const W = 1440, H = 900, FPS = 25

const findings = (dir) => {
  const p = join(run, dir, 'report.md')
  if (!existsSync(p)) return null
  const text = readFileSync(p, 'utf8')
  const count = (re) => (text.match(re) ?? []).length
  return { p0: count(/Severity:\s*P0/g), p1: count(/Severity:\s*P1/g), p2: count(/Severity:\s*P2/g), p3: count(/Severity:\s*P3/g) }
}

const browser = await chromium.launch()
const page = await browser.newPage({ viewport: { width: W, height: H } })
async function card(name, title, lines) {
  await page.setContent(`<html><body style="margin:0;width:${W}px;height:${H}px;display:flex;flex-direction:column;justify-content:center;padding:0 120px;box-sizing:border-box;background:#0a0a0a;color:#fafafa;font-family:-apple-system,system-ui,sans-serif">
    <div style="font-size:18px;letter-spacing:.12em;color:#ff4081;font-weight:700">QA · ${new Date().toISOString().slice(0, 10)}</div>
    <div style="font-size:56px;font-weight:700;margin:18px 0 28px">${title}</div>
    ${lines.map((l) => `<div style="font-size:24px;color:#a3a3a3;margin:6px 0">${l}</div>`).join('')}
  </body></html>`)
  const png = join(work, `${name}.png`)
  await page.screenshot({ path: png })
  const mp4 = join(work, `${name}.mp4`)
  execFileSync('ffmpeg', ['-v', 'error', '-y', '-loop', '1', '-t', '3.5', '-i', png, '-f', 'lavfi', '-t', '3.5', '-i', 'anullsrc=r=44100:cl=stereo',
    '-vf', `fps=${FPS},format=yuv420p`, '-c:v', 'libx264', '-preset', 'veryfast', '-c:a', 'aac', '-shortest', mp4])
  return mp4
}
function normalise(src, name) {
  const mp4 = join(work, `${name}.mp4`)
  execFileSync('ffmpeg', ['-v', 'error', '-y', '-i', src, '-f', 'lavfi', '-i', 'anullsrc=r=44100:cl=stereo',
    '-vf', `scale=${W}:${H}:force_original_aspect_ratio=decrease,pad=${W}:${H}:(ow-iw)/2:(oh-ih)/2:color=0x0a0a0a,fps=${FPS},format=yuv420p`,
    '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '24', '-c:a', 'aac', '-shortest', mp4])
  return mp4
}

const parts = []
const totals = { p0: 0, p1: 0, p2: 0, p3: 0 }
const clipsByGroup = groups.map(({ dir, title }) => {
  const vids = existsSync(join(run, dir, 'videos')) ? readdirSync(join(run, dir, 'videos')).filter((f) => f.endsWith('.webm')).sort() : []
  const f = findings(dir)
  if (f) for (const k of Object.keys(totals)) totals[k] += f[k]
  return { dir, title, vids, f }
})
parts.push(await card('00-intro', 'ADE end-to-end QA', [
  'A brand-new user\'s stack, built from this branch',
  `${clipsByGroup.reduce((n, g) => n + g.vids.length, 0)} recorded journeys in ${groups.length} groups`,
  ...clipsByGroup.map((g) => `• ${g.title}`),
]))
for (const [gi, g] of clipsByGroup.entries()) {
  parts.push(await card(`${gi + 1}0-group`, g.title, [
    `${g.vids.length} ${g.vids.length === 1 ? 'journey' : 'journeys'}`,
    g.f ? `Findings: ${g.f.p0} P0 · ${g.f.p1} P1 · ${g.f.p2} P2 · ${g.f.p3} P3` : 'No report found',
  ]))
  for (const [vi, v] of g.vids.entries()) {
    console.log('clip', g.dir, v)
    parts.push(normalise(join(run, g.dir, 'videos', v), `${gi + 1}${String(vi + 1).padStart(3, '0')}-clip`))
  }
}
parts.push(await card('99-outro', 'Results', [
  `P0 ${totals.p0} · P1 ${totals.p1} · P2 ${totals.p2} · P3 ${totals.p3}`,
  `Full report: ${join(run, 'report.md')}`,
]))
await browser.close()

const list = join(work, 'list.txt')
writeFileSync(list, parts.map((p) => `file '${p}'`).join('\n'))
const out = join(run, 'qa-video.mp4')
execFileSync('ffmpeg', ['-v', 'error', '-y', '-f', 'concat', '-safe', '0', '-i', list, '-c', 'copy', '-movflags', '+faststart', out])
const secs = execFileSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', out]).toString().trim()
console.log(`wrote ${out} (${Math.round(Number(secs))} s, ${parts.length} parts)`)
