import { chromium } from '@playwright/test'
import { execSync } from 'node:child_process'
const dir = '/tmp/fntest/profile'
const ctx = await chromium.launchPersistentContext(dir, { channel: 'chrome', headless: false })
const page = ctx.pages()[0] ?? await ctx.newPage()
await page.setContent('<input id=i autofocus><pre id=log></pre><script>for (const t of ["keydown","keyup"]) addEventListener(t, e => { log.textContent += t+" key="+e.key+" code="+e.code+" fn="+e.getModifierState("Fn")+"\\n" }, true)</script>')
await page.bringToFront(); await page.focus('#i'); await page.waitForTimeout(800)
const pid = execSync(`pgrep -f "user-data-dir=${dir}" | head -1`).toString().trim()
const ps = execSync(`ps -o command= -p ${pid}`).toString()
console.log('pid', pid, ps.includes('--type=') ? 'CHILD?' : 'main')
console.log(execSync(`/tmp/fntest/post ${pid}`).toString())
await page.waitForTimeout(800)
console.log(await page.textContent('#log'))
await ctx.close()
