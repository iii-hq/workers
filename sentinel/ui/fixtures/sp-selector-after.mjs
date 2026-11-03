import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import assert from 'node:assert/strict'
const require = createRequire(new URL('../../../ade/web/package.json', import.meta.url))
const { createServer } = await import(require.resolve('vite'))
const { chromium, expect } = require('@playwright/test')
const server = await createServer({ configFile: fileURLToPath(new URL('./vite.config.mjs', import.meta.url)) })
const evidence = process.env.SENTINEL_UI_EVIDENCE_DIR || fileURLToPath(new URL('../../docs/evidence/group-investigation-a73c', import.meta.url))
await mkdir(evidence, { recursive: true })
const observations = []
let browser
const bounds = locator => locator.evaluateAll(nodes => nodes.map(node => { const box = node.getBoundingClientRect(); return { x:box.x,y:box.y,width:box.width,height:box.height,right:box.right,bottom:box.bottom,scrollWidth:node.scrollWidth,clientWidth:node.clientWidth,scrollHeight:node.scrollHeight,clientHeight:node.clientHeight,textLength:node.textContent.length } }))
try {
  if (process.env.SENTINEL_UI_FORCE_EARLY_FAILURE === '1') throw new Error('Synthetic early fixture failure')
  await server.listen()
  browser = await chromium.launch({ headless:true, executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH })
  for (const theme of ['dark','light']) for (const scenario of [{name:'wide',width:1440,height:1000},{name:'phone',width:390,height:844},{name:'pane520',width:1440,height:1000,pane:520}]) {
    const page = await browser.newPage({viewport:{width:scenario.width,height:scenario.height},reducedMotion:'reduce'})
    page.setDefaultTimeout(6000)
    const errors=[]
    page.on('pageerror',error=>errors.push(String(error)))
    await page.goto(`http://127.0.0.1:4197/?selector=long&theme=${theme}${scenario.pane?`&pane=${scenario.pane}`:''}`)
    for (const title of ['TimeoutError: upstream request exceeded deadline','ConnectionError: socket closed during response']) await page.getByRole('checkbox',{name:`Select ${title}`,exact:true}).check()
    await page.getByRole('button',{name:'Investigate in group',exact:true}).click()
    const dialog=page.getByRole('dialog',{name:'Investigate in group',exact:true})
    const trigger=dialog.getByRole('combobox',{name:'Investigation profile',exact:true})
    await trigger.click()
    const list=page.getByRole('listbox',{name:'Installed investigation profiles',exact:true})
    const options=list.getByRole('option')
    await expect(options).toHaveCount(15)
    await list.scrollIntoViewIfNeeded()
    const measured={list:await bounds(list),options:await bounds(options),dialog:await bounds(dialog)}
    const box=measured.list[0], host=measured.dialog[0]
    console.log(JSON.stringify({ phase:'measured',theme,scenario,measured }))
    assert.ok(box.x>=host.x&&box.right<=host.right+1)
    assert.ok(box.width<=Math.min(480,scenario.pane||scenario.width))
    assert.ok(box.height<=241&&box.scrollHeight>box.clientHeight,'list height bounded and scrollable')
    assert.ok(box.x>=0&&box.right<=scenario.width&&box.y>=0&&box.bottom<=scenario.height)
    for (const option of measured.options) {
      assert.ok(option.x>=box.x&&option.right<=box.right+1,'each option horizontally inside list')
      assert.ok(option.height<=201&&option.scrollWidth<=option.clientWidth+1,'each option bounded; long data wraps')
    }
    assert.equal(await options.first().locator('.sentinel-ui-profile-description').textContent(),'SyntheticDescription'.repeat(175),'full long description retained')
    await expect(options.filter({hasText:'Broken chain'})).toBeDisabled()
    await page.screenshot({path:`${evidence}/cr-after-${scenario.name}-${theme}.png`,fullPage:true})
    // Ordinary Playwright click: no force, injected click, or coordinates.
    await options.filter({hasText:'Default · default'}).click()
    await expect(trigger).toBeFocused()
    await expect(dialog.getByRole('button',{name:'Start investigation',exact:true})).toBeEnabled()
    await trigger.click()
    await dialog.getByRole('heading',{name:'Investigate in group',exact:true}).click()
    await expect(list).toBeHidden()
    await expect(dialog).toBeVisible()
    // Native keyboard traverses options, confirms long identity, and restores focus.
    await trigger.focus()
    await page.keyboard.press('ArrowDown')
    await expect(options.filter({hasText:'Default · default'})).toBeFocused()
    await page.keyboard.press('Home')
    await expect(options.first()).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(trigger).toBeFocused()
    assert.ok((await trigger.innerText()).includes('Synthetic specialist'))
    const triggerBox=(await bounds(trigger))[0]
    assert.ok(triggerBox.width<=480&&triggerBox.height<=97)
    await trigger.click()
    await page.keyboard.press('End')
    await expect(options.filter({hasText:'Synthetic specialist 11'})).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(list).toBeHidden()
    await expect(trigger).toBeFocused()
    await expect(dialog).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(dialog).toBeHidden()
    await expect(page.getByText('2 selected',{exact:true})).toBeVisible()
    await page.getByRole('button',{name:'Investigate in group',exact:true}).click()
    await trigger.click()
    await options.filter({hasText:'Default · default'}).click()
    await dialog.getByRole('button',{name:'Cancel',exact:true}).click()
    await expect(page.getByText('2 selected',{exact:true})).toBeVisible()
    assert.deepEqual(await page.evaluate(()=>window.__SENTINEL_FIXTURE__.calls.filter(call=>call.id==='sentinel::investigate_many')),[])
    assert.deepEqual(errors,[])
    observations.push({theme,scenario,measured,triggerBox,normalClickDefault:true,keyboard:'ArrowDown/Home/Enter/End; first Escape closes list/restores focus; second closes dialog; disabled skipped',cancelPreserved:true,errors})
    await page.close()
  }
  const hashes={}
  for(const file of ['../src/page/InvestigateMany.tsx','../src/page/ProfilePicker.tsx','../styles.css','./sp-selector-data.ts']) hashes[file]=createHash('sha256').update(await readFile(new URL(file,import.meta.url))).digest('hex')
  await writeFile(`${evidence}/cr-after.json`,JSON.stringify({syntheticOnly:true,head:'9eb9b507598a0dbe0937b883aebe89ebcba7f6bb',reducedMotion:true,hashes,observations,pixelReview:'not performed by this script'},null,2))
  console.log(JSON.stringify({ok:true,scenarios:observations.length,hashes,observations:observations.map(({theme,scenario,normalClickDefault,errors})=>({theme,scenario,normalClickDefault,errors}))}))
} catch(error) { await writeFile(`${evidence}/cr-after-failure.json`,JSON.stringify({error:String(error),stack:error.stack,observations},null,2));throw error }
finally {await browser?.close();await server.close()}
