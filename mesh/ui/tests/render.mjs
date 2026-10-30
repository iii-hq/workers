import { strict as assert } from 'node:assert'
import { randomBytes, randomUUID } from 'node:crypto'
import { readFile, mkdir, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'
import { createServer } from 'node:net'
import { setTimeout as delay } from 'node:timers/promises'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const require = createRequire(resolve(root, 'ade/web/package.json'))
const { chromium, expect } = require('@playwright/test')
const artifacts = process.env.MESH_UI_ARTIFACTS ?? '/tmp/mesh-task22-render'
const opaqueOnly = process.env.MESH_UI_SCENARIO === 'opaque'
const focusOnly = process.env.MESH_UI_SCENARIO === 'focus'
const opaqueCase = process.env.MESH_UI_OPAQUE_CASE
await mkdir(artifacts, { recursive: true })
const portProbe = createServer()
await new Promise((resolve) => portProbe.listen(0, '127.0.0.1', resolve))
const port = Number(process.env.MESH_UI_TEST_PORT ?? portProbe.address().port)
await new Promise((resolve) => portProbe.close(resolve))
const url = `http://127.0.0.1:${port}`
// The isolated Console SPA uses its real loader/components; only the bus data is controlled.
const vite = spawn(process.execPath, [resolve(root, 'ade/web/node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: resolve(root, 'ade/web'), stdio: ['ignore', 'pipe', 'pipe'],
  env: { ...process.env, III_ENGINE_URL: 'ws://127.0.0.1:49853', III_CONSOLE_URL: 'http://127.0.0.1:49854' },
})
let viteLog = ''
vite.stdout.on('data', (b) => { viteLog += b })
vite.stderr.on('data', (b) => { viteLog += b })
let browser
try {
  let ready = false
  for (let attempt = 0; attempt < 120; attempt++) {
    try { if ((await fetch(url)).ok) { ready = true; break } } catch {}
    if (vite.exitCode !== null) throw new Error(`Vite exited ${vite.exitCode}: ${viteLog}`)
    await delay(250)
  }
  assert(ready, 'isolated Vite server started')
  browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH ?? '/usr/bin/chromium', headless: true })
  const codeRaw = [...randomBytes(8)].map((n) => '0123456789ABCDEFGHJKMNPQRSTVWXYZ'[n % 32]).join('')
  const joinCode = `${codeRaw.slice(0, 4)}-${codeRaw.slice(4)}`
  const unix = `curl -fsSL https://raw.githubusercontent.com/iii-hq/workers/mesh/v0.1.0/mesh/install/install.sh | sh -s -- 0.1.0 https://mesh.example.com ${joinCode}`
  const windows = `& ([scriptblock]::Create((irm https://raw.githubusercontent.com/iii-hq/workers/mesh/v0.1.0/mesh/install/install.ps1))) 0.1.0 https://mesh.example.com ${joinCode}`
  const schema = { type: 'object', properties: {
    public_url: { type: 'string' }, listen: { type: 'string' }, gateway_port: { type: 'integer', minimum: 1, maximum: 65535 },
    proxy_url: { type: 'string' }, stun_port: { type: 'integer', minimum: 0, maximum: 65535 },
    trusted_proxies: { type: 'array', items: { type: 'string' } }, links: { type: 'array' }, nodes: { type: 'object' }, drives: { type: 'object' },
  } }
  const entry = { id: 'mesh-lab', name: 'Mesh', description: 'Mesh hub networking, nodes and drives', schema, metadata: { ui_form: 'mesh' } }
  const initialConfig = { public_url: '', listen: '${MESH_LISTEN:0.0.0.0:49180}', gateway_port: 49135, proxy_url: 'ws://127.0.0.1:49200', stun_port: 0, trusted_proxies: [], links: [], nodes: {}, drives: {} }
  const nodes = [
    { name: 'linux-workstation', address: 'fd7a:115c:a1e0::2', os: 'linux', arch: 'amd64', version: '0.1.0', online: true, path: 'direct', workers: [{ name: 'ide', state: 'running' }], drives: [{ name: 'project', state: 'mounted' }] },
    { name: 'travel-laptop', address: 'fd7a:115c:a1e0::3', os: 'darwin', arch: 'arm64', version: '0.0.9', online: false, last_seen: new Date(Date.now() - 3_600_000).toISOString(), path: 'relay', workers: [], drives: [] },
  ]
  const longNodes = [{ ...nodes[0], name: 'engineering-workstation-archive', path: 'relay / ' + 'long-network-path/'.repeat(12), version: '0.1.0-build-' + '1234567890'.repeat(8), workers: [{ name: 'worker-with-a-very-long-name', state: 'waiting for the saved operator to sign in' }], drives: [{ name: 'long-shared-project-directory', state: 'mount error: ' + 'permission denied on remote directory '.repeat(5) }] }]
  const results = []
  for (const viewport of [{ width: 1440, height: 1000 }, { width: 390, height: 844 }]) {
    for (const theme of ['light', 'dark']) {
      const context = await browser.newContext({ viewport, permissions: ['clipboard-read', 'clipboard-write'], reducedMotion: 'reduce' })
      const page = await context.newPage()
      const errors = []
      page.on('pageerror', (err) => errors.push(String(err)))
      page.on('console', (msg) => { if (msg.type() === 'error' && msg.text().includes('[iii-ui]')) errors.push(msg.text()) })
      await page.addInitScript((theme) => { localStorage.setItem('iii-theme', theme) }, theme)
      await page.route('**/runtime', (route) => route.fulfill({ json: { namespace: 'project' } }))
      for (const asset of ['page.js', 'styles.css']) {
        const body = await readFile(resolve(root, 'mesh/assets', asset))
        await page.route(`**/ui/mesh/${asset}*`, (route) => route.fulfill({ body, contentType: asset.endsWith('.js') ? 'text/javascript' : 'text/css' }))
      }
      let mode = 'loading'
      const waiting = []
      const saved = []
      const calls = []
      let config = structuredClone(initialConfig)
      let activeSchema = schema
      await page.routeWebSocket('**/ws', (socket) => {
        socket.onMessage(async (message) => {
          const frame = JSON.parse(String(message))
          if (frame.type === 'registertrigger' && frame.trigger_type === 'console:assets') {
            socket.send(JSON.stringify({ type: 'invokefunction', invocation_id: randomUUID(), function_id: frame.function_id,
              data: { event: 'sync', assets: [{ path: 'mesh/styles.css', kind: 'style', hash: 'fixture-css' }, { path: 'mesh/page.js', kind: 'script', hash: 'fixture-js' }] } }))
          }
          if (frame.type !== 'invokefunction') return
          calls.push({ id: frame.function_id, namespace: frame.namespace, data: frame.data })
          const send = (result, error) => socket.send(JSON.stringify({ type: 'invocationresult', invocation_id: frame.invocation_id, function_id: frame.function_id, ...(error ? { error: { code: 'UNAVAILABLE', message: error } } : { result }) }))
          if (frame.function_id.startsWith('mesh::') && frame.function_id !== 'mesh::configuration-id' && mode === 'loading') {
            waiting.push(() => send(frame.function_id === 'mesh::nodes::list' ? [] : { code: joinCode, install: { unix, windows } }))
            return
          }
          switch (frame.function_id) {
            case 'mesh::configuration-id': assert.equal(frame.namespace, 'project'); return send({ id: 'mesh-lab' })
            case 'mesh::nodes::list': return mode === 'error' ? send(null, 'Mesh hub is unavailable. Reconnect, then retry.') : send(mode === 'empty' ? [] : mode === 'long' ? longNodes : nodes)
            case 'mesh::join::code': return mode === 'error' || mode === 'join-error' ? send(null, 'public_url is not set') : send({ code: joinCode, install: { unix, windows } })
            case 'configuration::list': return send({ configurations: [{ ...entry, schema: activeSchema }] })
            case 'configuration::schema': assert.equal(frame.data.id, 'mesh-lab'); return send({ ...entry, schema: activeSchema })
            case 'configuration::get': assert.equal(frame.data.id, 'mesh-lab'); return send({ id: 'mesh-lab', value: config })
            case 'configuration::set': assert.equal(frame.data.id, 'mesh-lab'); saved.push(frame.data); config = frame.data.value; return send({ new_value: config, old_value: initialConfig })
            case 'engine::functions::list': return send({ functions: [] })
            case 'console::ui-manifest': return send({ warnings: [], disabled: false })
            case 'harness::sessions::list': return send({ sessions: [] })
            default: return send({})
          }
        })
      })
      const capture = async (name) => {
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false, 'no page overflow')
        assert.deepEqual(errors, [], 'no uncaught render/injection errors')
        const measurements = await page.locator('.mesh-ui-main').evaluate((el) => ({ width: el.clientWidth, scrollWidth: el.scrollWidth }))
        assert(measurements.scrollWidth <= measurements.width, 'no content overflow inside the pane')
        const path = resolve(artifacts, `${viewport.width}-${theme}-${name}.png`)
        await page.screenshot({ path })
        results.push(path)
      }
      if (!opaqueOnly && !focusOnly) {
        await page.goto(`${url}/#/worker/mesh/mesh-manager`)
        await expect(page.getByText('Loading machines and join instructions…')).toBeVisible()
        await capture('loading')
        await page.keyboard.press('Tab')
        await expect(page.getByRole('button', { name: 'Settings', exact: true })).toBeFocused()
        mode = 'empty'; waiting.splice(0).forEach((send) => send())
        await expect(page.getByText('No machines joined', { exact: true })).toBeVisible()
        await capture('empty')
        mode = 'error'; await page.getByRole('button', { name: 'Refresh', exact: true }).click()
        await expect(page.getByText('Could not load machines')).toBeVisible()
        await expect(page.getByText('Join instructions unavailable')).toBeVisible()
        await capture('error')
        mode = 'populated'; await page.getByRole('button', { name: 'Retry', exact: true }).first().click()
        await expect(page.getByRole('heading', { name: 'linux-workstation' })).toBeVisible()
        await expect(page.getByText('Online', { exact: true })).toBeVisible()
        await expect(page.getByText('Offline', { exact: true })).toBeVisible()
        await capture('populated')
        await page.getByRole('button', { name: 'Copy Join code', exact: true }).click()
        await expect(page.getByText('Copied', { exact: true })).toBeVisible()
        assert.equal(await page.evaluate(() => navigator.clipboard.readText()), joinCode)
        await page.getByRole('button', { name: 'Copy Linux / macOS command', exact: true }).click()
        assert.equal(await page.evaluate(() => navigator.clipboard.readText()), unix)
        await page.getByRole('button', { name: 'Copy Windows PowerShell command', exact: true }).click()
        assert.equal(await page.evaluate(() => navigator.clipboard.readText()), windows)
        await capture('join-actions')
        mode = 'join-error'; await page.getByRole('button', { name: 'Refresh', exact: true }).click()
        await expect(page.getByText('Join instructions unavailable')).toBeVisible()
        await expect(page.getByRole('heading', { name: 'linux-workstation' })).toBeVisible()
        mode = 'long'; await page.getByRole('button', { name: 'Refresh', exact: true }).click()
        await expect(page.getByRole('heading', { name: longNodes[0].name })).toBeVisible()
        await capture('long')
        await page.getByRole('button', { name: 'Settings', exact: true }).click()
        await expect(page).toHaveURL(/#\/configuration\/workers\/mesh-lab$/)
        const publicUrl = page.getByRole('textbox', { name: 'Public URL', exact: true })
        await expect(publicUrl).toBeVisible()
        await publicUrl.fill('https://mesh.example.com')
        await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toBeEnabled()
        // Reset an unsaved draft so reset and persistence checks have independent baselines.
        const configCallsBeforeEdit = calls.filter((call) => call.id.startsWith('configuration::'))
        await publicUrl.fill('https://changed.example.com')
        try {
          await expect(publicUrl).toHaveValue('https://changed.example.com')
          await expect(page.getByRole('button', { name: 'Reset', exact: true })).toBeVisible()
        } catch (error) {
          await page.screenshot({ path: resolve(artifacts, 'reset-diagnostic.png') })
          await writeFile(resolve(artifacts, 'reset-diagnostic.json'), JSON.stringify({ before: configCallsBeforeEdit, after: calls.filter((call) => call.id.startsWith('configuration::')), value: await publicUrl.inputValue(), text: await page.locator('body').innerText() }, null, 2))
          throw error
        }
        await page.getByRole('button', { name: 'Reset', exact: true }).click()
        await expect(publicUrl).toHaveValue('')
        await publicUrl.fill('https://mesh.example.com')
        await page.getByRole('textbox', { name: 'Gateway port', exact: true }).fill('0')
        await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toBeDisabled()
        await page.getByRole('textbox', { name: 'Gateway port', exact: true }).fill('49135')
        await page.getByRole('textbox', { name: 'New machine name', exact: true }).fill('constructor')
        await expect(page.getByRole('button', { name: 'Add machine', exact: true })).toBeEnabled()
        await page.getByRole('textbox', { name: 'New machine name', exact: true }).fill('laptop')
        await page.getByRole('button', { name: 'Add machine', exact: true }).click()
        await page.getByRole('region', { name: 'Workers', exact: true }).getByRole('button', { name: 'Add item' }).click()
        await page.getByRole('textbox', { name: 'Workers 1', exact: true }).fill('ide')
        await page.getByRole('textbox', { name: 'New drive name', exact: true }).fill('project')
        await page.getByRole('button', { name: 'Add drive', exact: true }).click()
        await page.getByRole('textbox', { name: 'Server machine', exact: true }).fill('laptop')
        await page.getByRole('region', { name: 'Client machines', exact: true }).getByRole('button', { name: 'Add item' }).click()
        await page.getByRole('textbox', { name: 'Client machines 1', exact: true }).fill('*')
        await page.getByText('Advanced networking', { exact: true }).click()
        await page.getByRole('button', { name: 'Add link', exact: true }).click()
        await page.getByRole('textbox', { name: 'Link 1 first machine', exact: true }).fill('laptop')
        await page.getByRole('textbox', { name: 'Link 1 second machine', exact: true }).fill('desktop')
        await capture('configuration')
        await page.getByRole('button', { name: 'Save changes', exact: true }).click()
        await expect.poll(() => saved.length).toBe(1)
        await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toHaveCount(0)
        assert.equal(saved[0].value.listen, initialConfig.listen, 'unrelated environment template preserved')
        assert.deepEqual(saved[0].value.nodes.laptop.workers, ['ide'])
        assert.deepEqual(saved[0].value.drives.project, { server: 'laptop', nodes: ['*'] })
        assert.deepEqual(saved[0].value.links, [['laptop', 'desktop']])
        assert(calls.some((call) => call.id === 'configuration::get' && call.data.raw === true && call.namespace === 'default'))
        assert(calls.every((call) => !call.id.startsWith('configuration::') || call.namespace === 'default'))
        assert.deepEqual(errors, [])
        console.log(`PASS ${viewport.width}px ${theme}: loader, states, overflow, clipboard, dynamic identity, host validation/save/reset, templates, node/drive/link edits`)
      }
      const openDraft = async (value) => {
        config = structuredClone(value)
        // A fresh document gives each draft its own host baseline/query cache.
        // An unconstrained fixture schema lets the host serialize pre-existing custom shapes.
        activeSchema = {}
        mode = 'populated'
        await page.goto(`${url}/?draft=${randomUUID()}#/worker/mesh/mesh-manager`)
        try {
          await page.getByRole('button', { name: 'Settings', exact: true }).click()
        } catch (error) {
          await writeFile(resolve(artifacts, 'host-open-draft-diagnostic.json'), JSON.stringify({ errors, calls, body: await page.locator('body').innerText() }, null, 2))
          throw error
        }
        await expect(page).toHaveURL(/#\/configuration\/workers\/mesh-lab$/)
      }
      const persist = async (expected) => {
        const count = saved.length
        await page.getByRole('button', { name: 'Save changes', exact: true }).click()
        await expect.poll(() => saved.length).toBe(count + 1)
        await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toHaveCount(0)
        assert.deepEqual(saved.at(-1).value, expected, 'host receives the entire preserved draft')
      }
      const replace = (label) => page.getByRole('button', { name: `Replace ${label} raw value with a literal value`, exact: true })
      const unchanged = async () => {
        await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toHaveCount(0)
      }
      if (!opaqueOnly) {
        const focusDraft = { ...initialConfig, trusted_proxies: ['127.0.0.1/32'], links: [['laptop', 'desktop']], nodes: { laptop: { workers: ['ide'] } }, drives: { project: { server: 'laptop', nodes: ['*'] } } }
        for (const field of ['trusted_proxies.0', 'links.0.0', 'nodes.laptop.workers.0', 'drives.project.server']) {
          await openDraft(focusDraft)
          const target = page.locator(`[data-field="${field}"]`)
          await expect(target).toHaveCount(1)
          const disclosure = target.locator('xpath=ancestor::details[1]')
          if (field.startsWith('nodes.') || field.startsWith('drives.')) {
            await disclosure.locator('summary').click()
          }
          await expect(disclosure).not.toHaveAttribute('open', '')
          await expect(target).toBeHidden()
          await page.evaluate((field) => { window.location.hash = `/configuration/workers/mesh-lab/${field.split('.').join('/')}` }, field)
          await expect(target).toBeVisible()
          await expect(target).toBeFocused()
          await expect(disclosure).toHaveAttribute('open', '')
          await unchanged()
          await capture(`focus-${field}`)
        }
        console.log(`PASS ${viewport.width}px ${theme}: deep links reveal and focus closed advanced settings and manually collapsed machine/drive entries`)
      }
      if (focusOnly) {
        await context.close()
        continue
      }
      if (!opaqueCase || opaqueCase === 'root') {
        await openDraft('${MESH_CONFIG}')
        await expect(page.getByRole('textbox', { name: 'Public URL', exact: true })).toHaveCount(0)
        const rawRoot = page.getByRole('textbox', { name: 'Mesh configuration', exact: true })
        await expect(rawRoot).toHaveValue('${MESH_CONFIG}')
        await unchanged()
        await capture('opaque-root')
        await rawRoot.fill('${OTHER_MESH_CONFIG}')
        await persist('${OTHER_MESH_CONFIG}')
        await openDraft('${MESH_CONFIG}')
        await replace('Mesh configuration').click()
        await page.getByRole('textbox', { name: 'Public URL', exact: true }).fill('https://mesh.example.com')
        await persist({ ...initialConfig, listen: '0.0.0.0:49180', public_url: 'https://mesh.example.com' })
        console.log(`PASS ${viewport.width}px ${theme}: opaque root remains raw until explicit conversion; complete raw/default drafts round-trip through host`)
      }
      if (!opaqueCase || opaqueCase === 'link') {
        const linkDraft = { ...initialConfig, links: ['${MESH_LINK}', ['alpha', 'beta']], unrelated: '${KEEP_THIS}' }
        await openDraft(linkDraft)
        await page.getByText('Advanced networking', { exact: true }).click()
        await expect(page.getByRole('textbox', { name: 'Link 1 first machine', exact: true })).toHaveCount(0)
        const rawLink = page.getByRole('textbox', { name: 'Link 1', exact: true })
        await expect(rawLink).toHaveValue('${MESH_LINK}')
        await rawLink.scrollIntoViewIfNeeded()
        await unchanged()
        await capture('opaque-link')
        await page.getByRole('textbox', { name: 'Public URL', exact: true }).fill('https://mesh.example.com')
        await persist({ ...linkDraft, public_url: 'https://mesh.example.com' })
        await openDraft(linkDraft)
        await page.getByText('Advanced networking', { exact: true }).click()
        await replace('Link 1').click()
        await page.getByRole('textbox', { name: 'Link 1 first machine', exact: true }).fill('laptop')
        await page.getByRole('textbox', { name: 'Link 1 second machine', exact: true }).fill('desktop')
        await persist({ ...linkDraft, links: [['laptop', 'desktop'], ['alpha', 'beta']] })
        console.log(`PASS ${viewport.width}px ${theme}: opaque link and siblings survive unrelated edits; explicit conversion creates an array pair`)
      }
      if (!opaqueCase && viewport.width === 1440 && theme === 'light') {
        for (const value of [null, false, 42, ['${ROOT_ITEM}']]) {
          await openDraft(value)
          const raw = page.getByRole('textbox', { name: 'Mesh configuration', exact: true })
          await expect(raw).toHaveValue(JSON.stringify(value))
          await expect(raw).toHaveJSProperty('readOnly', true)
          await expect(page.getByRole('textbox', { name: 'Public URL', exact: true })).toHaveCount(0)
          await unchanged()
          if (value === 42) await capture('custom-root')
          await replace('Mesh configuration').click()
          await persist({ ...initialConfig, listen: '0.0.0.0:49180' })
        }
        const boundaries = [
          { name: 'Machine workers', patch: { nodes: '${MESH_NODES}' }, after: { nodes: {} } },
          { name: 'Shared drives', patch: { drives: ['${DRIVE_ITEM}'] }, after: { drives: {} } },
          { name: 'Machine laptop settings', patch: { nodes: { laptop: '${LAPTOP}', other: '${OTHER_NODE}' } }, after: { nodes: { laptop: { workers: [], allow: [] }, other: '${OTHER_NODE}' } } },
          { name: 'Drive archive settings', patch: { drives: { archive: false, other: '${OTHER_DRIVE}' } }, after: { drives: { archive: { server: '', nodes: [] }, other: '${OTHER_DRIVE}' } } },
          { name: 'Workers', patch: { nodes: { laptop: { workers: { custom: '${WORKERS}' }, allow: ['${GRANT}'] } } }, after: { nodes: { laptop: { workers: [], allow: ['${GRANT}'] } } } },
          { name: 'Workers 1', patch: { nodes: { laptop: { workers: [false, 'ide'], allow: [] } } }, after: { nodes: { laptop: { workers: ['', 'ide'], allow: [] } } } },
          { name: 'Client machines', patch: { drives: { archive: { server: 'laptop', nodes: '${DRIVE_CLIENTS}' } } }, after: { drives: { archive: { server: 'laptop', nodes: [] } } } },
          { name: 'Trusted proxies', advanced: true, patch: { trusted_proxies: null }, after: { trusted_proxies: [] } },
          { name: 'Machine links', advanced: true, patch: { links: { custom: '${LINKS}' } }, after: { links: [] } },
          { name: 'Link 1', advanced: true, patch: { links: [{ custom: '${PAIR}' }, ['alpha', 'beta']] }, after: { links: [['', ''], ['alpha', 'beta']] } },
          { name: 'Public URL', patch: { public_url: { custom: '${URL}' } }, after: { public_url: '' } },
          { name: 'Gateway port', patch: { gateway_port: '${GATEWAY_PORT}' }, after: { gateway_port: 49135 } },
        ]
        for (const boundary of boundaries) {
          const draft = { ...initialConfig, ...boundary.patch, unrelated: '${KEEP_THIS}' }
          await openDraft(draft)
          if (boundary.advanced) await page.getByText('Advanced networking', { exact: true }).click()
          const raw = page.getByRole('textbox', { name: boundary.name, exact: true })
          await expect(raw).toBeVisible()
          await unchanged()
          if (boundary.name === 'Drive archive settings') {
            await raw.scrollIntoViewIfNeeded()
            await capture('custom-drive')
          }
          if (boundary.name !== 'Public URL') {
            await page.getByRole('textbox', { name: 'Public URL', exact: true }).fill('https://mesh.example.com')
            await persist({ ...draft, public_url: 'https://mesh.example.com' })
            await openDraft(draft)
            if (boundary.advanced) await page.getByText('Advanced networking', { exact: true }).click()
          }
          await replace(boundary.name).click()
          await persist({ ...draft, ...boundary.after })
        }
        const absent = { public_url: '', listen: '${KEEP_LISTENER}' }
        await openDraft(absent)
        await unchanged()
        await page.getByRole('textbox', { name: 'Public URL', exact: true }).fill('https://mesh.example.com')
        await persist({ ...absent, public_url: 'https://mesh.example.com' })
        await openDraft(absent)
        await page.getByRole('textbox', { name: 'New machine name', exact: true }).fill('laptop')
        await page.getByRole('button', { name: 'Add machine', exact: true }).click()
        await persist({ ...absent, nodes: { laptop: { workers: [], allow: [] } } })
        console.log('PASS opaque shape matrix: 4 custom roots, 12 nested boundaries, absent optional parents, explicit conversion and complete sibling/template preservation')
      }
      assert.deepEqual(errors, [], 'opaque drafts render without host/loader errors')
      await context.close()
    }
  }
  await writeFile(resolve(artifacts, 'captures.json'), JSON.stringify(results, null, 2))
} finally {
  await browser?.close()
  vite.kill('SIGTERM')
  await writeFile(resolve(artifacts, 'vite.log'), viteLog)
}
