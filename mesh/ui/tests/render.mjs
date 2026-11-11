import { strict as assert } from 'node:assert'
import { randomBytes, randomUUID } from 'node:crypto'
import { existsSync } from 'node:fs'
import { readFile, mkdir, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'
import { createServer } from 'node:net'
import { setTimeout as delay } from 'node:timers/promises'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
// Playwright and Vite come from the Console SPA's install in the workspace pnpm store.
const require = createRequire(resolve(root, 'ade/web/package.json'))
const { chromium, expect } = require('@playwright/test')
// The bundled Playwright browser is not used: MESH_TEST_CHROMIUM_BIN (or the older PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH)
// names one; otherwise /usr/bin/chromium, then macOS system Chrome. No CI workflow runs this test yet.
const executablePath = [process.env.MESH_TEST_CHROMIUM_BIN, process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH, '/usr/bin/chromium', '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome']
  .find((path) => path && existsSync(path))
assert(executablePath, 'set MESH_TEST_CHROMIUM_BIN to a Chromium or Chrome executable')
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
  browser = await chromium.launch({ executablePath, headless: true })
  const codeRaw = [...randomBytes(8)].map((n) => '0123456789ABCDEFGHJKMNPQRSTVWXYZ'[n % 32]).join('')
  const joinCode = `${codeRaw.slice(0, 4)}-${codeRaw.slice(4)}`
  const tunnelURL = 'https://r0ute234abcd5678.abc234.opentunnel.xyz'
  const schema = { type: 'object', properties: {
    public_url: { type: 'string' }, listen: { type: 'string' }, gateway_port: { type: 'integer', minimum: 1, maximum: 65535 },
    proxy_url: { type: 'string' }, stun_port: { type: 'integer', minimum: 0, maximum: 65535 },
    trusted_proxies: { type: 'array', items: { type: 'string' } },
    links: { description: 'Retired on 2026-11-10; ignored.' }, nodes: { description: 'Retired on 2026-11-10; ignored.' }, drives: { description: 'Retired on 2026-11-10; ignored.' },
  } }
  const entry = { id: 'mesh-lab', name: 'Mesh', description: 'Mesh hub and client networking', schema, metadata: { ui_form: 'mesh' } }
  // What replacing a raw root writes: defaultConfiguration() in page.tsx.
  const defaults = { public_url: '', listen: '0.0.0.0:49180', gateway_port: 49135, proxy_url: 'ws://127.0.0.1:49200', stun_port: 0, trusted_proxies: [] }
  // An entry written before 2026-11-10: the page and the form must carry links, nodes and drives through untouched.
  const initialConfig = { public_url: '', listen: '${MESH_LISTEN:0.0.0.0:49180}', gateway_port: 49135, proxy_url: 'ws://127.0.0.1:49200', stun_port: 0, trusted_proxies: [], links: [], nodes: {}, drives: {} }
  // Offline first, and archive-laptop sorts before every online name: the page must list online machines first, then by name.
  const lastSeen = new Date(Date.now() - 3_600_000).toISOString()
  // Each shared worker carries its hub function ids, <machine>::<local id>, sorted. A local id need not start with its worker's
  // name: ide registers coder::* and shell::*.
  const nodes = [
    { name: 'archive-laptop', address: 'fd7a:115c:a1e0::3', os: 'darwin', arch: 'arm64', version: '0.0.9', online: false, last_seen: lastSeen, path: 'relay', workers: [] },
    // The hub keeps an offline machine's last reported workers; their functions left with its bridge.
    { name: 'travel-laptop', address: 'fd7a:115c:a1e0::4', os: 'darwin', arch: 'arm64', version: '0.1.0', online: false, last_seen: lastSeen, path: 'relay',
      workers: [{ name: 'mesh-probe', state: 'shared', functions: ['travel-laptop::mesh-probe::echo'] }] },
    { name: 'linux-workstation', address: 'fd7a:115c:a1e0::2', os: 'linux', arch: 'amd64', version: '0.1.0', online: true, path: 'direct',
      workers: [{ name: 'ide', state: 'shared', functions: ['linux-workstation::coder::read-file', 'linux-workstation::shell::exec'] }] },
    { name: 'gpu-box', address: 'fd7a:115c:a1e0::5', os: 'linux', arch: 'amd64', version: '0.1.0', online: true, path: 'direct',
      workers: [{ name: 'image-resize', state: 'shared', functions: ['gpu-box::image_resize::resize', 'gpu-box::image_resize::thumbnail'] }, { name: 'tmux', state: 'pending', functions: [] }] },
  ]
  // The hub engine's function list supplies the descriptions: shell::exec has none, image_resize::thumbnail has no row yet.
  const bridge = (machine) => `rbac-proxy:${machine}::mesh-bridge`
  const longFunction = { function_id: `engineering-workstation-archive::archive_${'x'.repeat(60)}::fetch`, worker_name: bridge('engineering-workstation-archive'),
    description: `Fetches https://api.example.com/v1/${'abcdef0123456789'.repeat(5)}` }
  const hubFunctions = [
    { function_id: 'linux-workstation::shell::exec', worker_name: bridge('linux-workstation') },
    { function_id: 'linux-workstation::coder::read-file', worker_name: bridge('linux-workstation'), description: 'Reads a file from the workspace' },
    { function_id: 'linux-workstation::mesh-node::status', worker_name: 'rbac-proxy:linux-workstation::mesh-node' },
    { function_id: 'gpu-box::image_resize::resize', worker_name: bridge('gpu-box'), description: 'Resizes an image' },
    longFunction,
  ]
  const longNodes = [{ ...nodes[2], name: 'engineering-workstation-archive', path: 'relay / ' + 'long-network-path/'.repeat(12), version: '0.1.0-build-' + '1234567890'.repeat(8),
    workers: [{ name: 'worker-with-a-very-long-name', state: 'waiting for the saved operator to sign in', functions: [longFunction.function_id] },
      { name: 'another-worker-with-a-long-name', state: 'shared', functions: [] }] }]
  // Local workers the client share list offers; mesh and secrets are never shareable.
  const localWorkers = ['shell', 'state', 'mesh', 'secrets']
  const results = []
  for (const viewport of [{ width: 1440, height: 1000 }, { width: 390, height: 844 }]) {
    for (const theme of ['light', 'dark']) {
      // The 5 s polling cadence is checked in real time once; other contexts refresh by hand.
      const timed = viewport.width === 1440 && theme === 'light'
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
      // What the running worker reports, as mesh::status (hub tunnelStatus, client status) would.
      let running = 'hub'
      let tunnelPhase = { state: 'starting' }
      let clientState = 'connected'
      // The entry the hub last applied; null follows every save at once. Unset ${VAR} templates expand to their default.
      let applied = null
      const hubStatus = () => {
        const hub = applied ?? config
        const publicURL = String(hub?.public_url ?? '').replace(/\$\{\w+:?([^}]*)\}/g, '$1')
        const tunnel = hub?.tunnel?.enabled !== true ? undefined
          : publicURL ? { enabled: true, tunnel_id: 'mesh', state: 'overridden' }
          : { enabled: true, tunnel_id: 'mesh', ...tunnelPhase }
        return { version: '0.1.0', public_url: publicURL || tunnel?.url || '', ...(tunnel ? { tunnel } : {}) }
      }
      // Calls named in `hold` wait in `held` until release() answers them.
      let hold = new Set()
      const held = []
      const release = () => held.splice(0).forEach((answer) => answer())
      const clientStatus = () => ({ mode: 'client', hub_url: config.client?.hub_url ?? '', node: config.client?.name ?? '', namespace: 'project',
        state: clientState, error: '', shared: [{ worker: 'shell', state: 'running', functions: ['shell::exec'] }] })
      await page.routeWebSocket('**/ws', (socket) => {
        socket.onMessage(async (message) => {
          const frame = JSON.parse(String(message))
          if (frame.type === 'invokefunction' && frame.function_id === 'engine::workers::register') {
            socket.send(JSON.stringify({ type: 'workerregistered', worker_id: randomUUID(), features: ['observe.v1', 'observe.v2', 'local-dispatch.v1'] }))
          }
          if (frame.type === 'registerfunction') socket.send(JSON.stringify({ type: 'functionregistered', id: frame.id, namespace: frame.namespace ?? 'project', revision: 0, local_dispatch: false, observers: [] }))
          if (frame.type === 'observe') socket.send(JSON.stringify({ type: 'observeresult', id: frame.id, state: 'active' }))
          if (frame.type === 'observe' && frame.source.function_id === 'ade::ui::on-assets-changed') {
            socket.send(JSON.stringify({ type: 'invokefunction', invocation_id: randomUUID(), function_id: frame.target.function_id,
              activation: { observer_id: frame.id }, data: { op: 'sync', assets: [{ path: 'mesh/styles.css', kind: 'style', hash: 'fixture-css' }, { path: 'mesh/page.js', kind: 'script', hash: 'fixture-js' }] } }))
          }
          if (frame.type !== 'invokefunction') return
          calls.push({ id: frame.function_id, namespace: frame.namespace, data: frame.data })
          const send = (result, error) => socket.send(JSON.stringify({ type: 'invocationresult', invocation_id: frame.invocation_id, function_id: frame.function_id, ...(error ? { error: { code: 'UNAVAILABLE', message: error } } : { result }) }))
          const answer = () => {
            switch (frame.function_id) {
              case 'mesh::configuration-id': assert.equal(frame.namespace, 'project'); return send({ id: 'mesh-lab' })
              case 'mesh::nodes::list': return mode === 'error' ? send(null, 'Mesh hub is unavailable. Reconnect, then retry.') : send(mode === 'empty' || mode === 'loading' ? [] : mode === 'long' ? longNodes : nodes)
              // Like the hub, no join code while there is no public URL (configured or from a ready tunnel).
              case 'mesh::join::code': return mode === 'error' || !hubStatus().public_url ? send(null, 'public_url is not set') : send({ code: joinCode })
              case 'mesh::status': return send(running === 'client' ? clientStatus() : hubStatus())
              case 'compose::restart': assert.deepEqual(frame.data, { container: 'mesh' }); running = config?.mode === 'client' ? 'client' : 'hub'; return send({})
              case 'configuration::list': return send({ configurations: [{ ...entry, schema: activeSchema }] })
              case 'configuration::schema': assert.equal(frame.data.id, 'mesh-lab'); return send({ ...entry, schema: activeSchema })
              case 'configuration::get': assert.equal(frame.data.id, 'mesh-lab'); return send({ id: 'mesh-lab', value: config })
              case 'configuration::set': assert.equal(frame.data.id, 'mesh-lab'); saved.push(frame.data); config = frame.data.value; return send({ new_value: config, old_value: initialConfig })
              case 'engine::functions::list': return mode === 'functions-error' ? send(null, 'Engine function list is unavailable.') : send({ functions: [...localWorkers.map((worker) => ({ function_id: `${worker}::fixture`, worker_name: worker })), ...hubFunctions] })
              case 'ade::ui-manifest': return send({ warnings: [], disabled: false })
              case 'harness::sessions::list': return send({ sessions: [] })
              default: return send({})
            }
          }
          // The page's first reads stay pending until the loading state has been checked.
          if (frame.function_id.startsWith('mesh::') && frame.function_id !== 'mesh::configuration-id' && mode === 'loading') return void waiting.push(answer)
          if (hold.has(frame.function_id)) return void held.push(answer)
          answer()
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
      const statusCalls = () => calls.filter((call) => call.id === 'mesh::status').length
      const button = (name) => page.getByRole('button', { name, exact: true })
      const textbox = (name) => page.getByRole('textbox', { name, exact: true })
      const tunnelSwitch = page.getByRole('switch', { name: 'Expose through a tunnel', exact: true })
      const tunnelLine = page.locator('.mesh-ui-tunnel')
      const saveBar = page.locator('.mesh-ui-save-bar')
      const badge = page.getByText('needed before machines can join', { exact: true })
      // Page Save: only the edited basics change; every other key, templates included, is written back as read.
      const savePage = async (expected) => {
        const count = saved.length
        await button('Save').click()
        await expect.poll(() => saved.length).toBe(count + 1)
        await expect(saveBar).toHaveCount(0)
        assert.deepEqual(saved.at(-1).value, expected, 'configuration::set receives the stored entry with only the changed keys')
        return expected
      }
      const refreshTo = async (phase) => {
        tunnelPhase = phase
        await button('Refresh').click()
      }
      const openPage = async () => {
        await page.goto(`${url}/#/worker/mesh/mesh-manager`)
        await expect(page.getByRole('radiogroup', { name: 'Mesh mode', exact: true })).toBeVisible()
      }
      if (!opaqueOnly && !focusOnly) {
        await page.goto(`${url}/#/worker/mesh/mesh-manager`)
        await expect(page.getByText('Loading machines and join instructions…')).toBeVisible()
        await capture('loading')
        await page.keyboard.press('Tab')
        await expect(button('Settings')).toBeFocused()
        mode = 'empty'; waiting.splice(0).forEach((answer) => answer())
        await expect(page.getByText('No machines joined', { exact: true })).toBeVisible()
        await capture('empty')
        mode = 'error'; await button('Refresh').click()
        await expect(page.getByText('Could not load machines')).toBeVisible()
        await expect(page.getByText('Join instructions unavailable')).toBeVisible()
        await capture('error')
        mode = 'populated'; await button('Retry').first().click()
        await expect(page.getByRole('heading', { name: 'linux-workstation' })).toBeVisible()
        const card = (name) => page.getByRole('article', { name, exact: true })
        const workstation = card('linux-workstation')
        await expect(workstation.getByText('Online', { exact: true })).toBeVisible()
        await expect(card('archive-laptop').getByText('Offline', { exact: true })).toBeVisible()
        await expect(page.locator('.mesh-ui-node h3')).toHaveText(['gpu-box', 'linux-workstation', 'archive-laptop', 'travel-laptop'])
        // Each machine shows what it shares: the worker, then the functions it lists, with the engine's descriptions.
        await expect(workstation.getByRole('list', { name: 'ide functions', exact: true }).getByRole('listitem')).toHaveText([
          'linux-workstation::coder::read-file Reads a file from the workspace', 'linux-workstation::shell::exec',
        ])
        await expect(workstation.getByText('Reads a file from the workspace', { exact: true })).toBeVisible()
        await expect(workstation.getByText('shared', { exact: true })).toBeVisible()
        const gpu = card('gpu-box')
        await expect(gpu.getByRole('list', { name: 'image-resize functions', exact: true }).getByRole('listitem')).toHaveText([
          'gpu-box::image_resize::resize Resizes an image', 'gpu-box::image_resize::thumbnail',
        ])
        await expect(gpu.getByRole('list', { name: 'tmux functions', exact: true })).toHaveCount(0)
        await expect(gpu.getByRole('listitem').filter({ hasText: 'tmux' }).getByText('No functions registered yet', { exact: true })).toBeVisible()
        await expect(card('archive-laptop').getByText('Not sharing any workers', { exact: true })).toBeVisible()
        const travel = card('travel-laptop')
        await expect(travel.getByRole('listitem').filter({ hasText: 'mesh-probe' }).getByText('Unavailable while the machine is offline', { exact: true })).toBeVisible()
        await expect(travel.getByText('travel-laptop::mesh-probe::echo')).toHaveCount(0)
        // The hub keeps an offline machine's last state; showing "shared" would contradict the line above.
        await expect(travel.getByText('shared', { exact: true })).toHaveCount(0)
        const headings = await page.getByRole('heading', { level: 2 }).allTextContents()
        assert(headings.includes('Machines') && headings.indexOf('Machines') < headings.indexOf('Hub address'), 'Machines comes before Hub address in hub mode')
        await capture('populated')
        // The hub has no public URL yet: no join code, and the machines stay listed.
        await expect(page.getByText('Join instructions unavailable')).toBeVisible()
        await expect(button('Copy Join code')).toHaveCount(0)
        await expect(button('Copy Hub URL')).toHaveCount(0)
        mode = 'long'; await button('Refresh').click()
        await expect(page.getByRole('heading', { name: longNodes[0].name })).toBeVisible()
        await expect(page.getByRole('list', { name: 'worker-with-a-very-long-name functions', exact: true }).getByRole('listitem')).toHaveText([`${longFunction.function_id} ${longFunction.description}`])
        await capture('long')
        // A failed function list loses only the descriptions: the hub's ids still show.
        mode = 'functions-error'; await button('Refresh').click()
        await expect(workstation.getByRole('list', { name: 'ide functions', exact: true }).getByRole('listitem')).toHaveText(['linux-workstation::coder::read-file', 'linux-workstation::shell::exec'])
        await expect(page.getByText(/Could not load functions/)).toHaveCount(0)
        mode = 'populated'; await button('Refresh').click()
        await expect(workstation.getByRole('list', { name: 'ide functions', exact: true })).toBeVisible()
        console.log(`PASS ${viewport.width}px ${theme}: loader, machine states, overflow, join refused without a public URL`)

        // Hub mode: the basics are on the page, client fields are not.
        mode = 'populated'
        await expect(page.getByRole('radio', { name: 'Hub', exact: true })).toBeChecked()
        await expect(page.getByText('This project is the hub. Other projects join it as clients and share workers here.')).toBeVisible()
        await expect(page.getByRole('heading', { name: 'Hub address', exact: true })).toBeVisible()
        await expect(badge).toBeVisible()
        await expect(tunnelSwitch).not.toBeChecked()
        await expect(textbox('Public URL')).toHaveValue('')
        await expect(tunnelLine).toHaveCount(0)
        await expect(textbox('Hub URL')).toHaveCount(0)
        await expect(page.getByRole('heading', { name: 'Shared workers' })).toHaveCount(0)
        await expect(saveBar).toHaveCount(0)
        await capture('hub')
        await textbox('Public URL').fill('https://mesh.example.com')
        await expect(saveBar).toContainText('Unsaved changes · applied within a few seconds')
        await button('Discard').click()
        await expect(saveBar).toHaveCount(0)
        await expect(textbox('Public URL')).toHaveValue('')
        assert.equal(saved.length, 0, 'Discard writes nothing')

        // The tunnel switch writes tunnel.enabled and the status line follows the hub's tunnel state.
        await tunnelSwitch.click()
        await expect(tunnelSwitch).toBeChecked()
        await expect(saveBar).toContainText('Unsaved changes · applied within a few seconds')
        // An edit typed while the save is in flight is kept, and the page shows what it wrote before it reads again.
        hold = new Set(['configuration::set'])
        await button('Save').click()
        await expect.poll(() => held.length).toBe(1)
        await textbox('Public URL').fill('https://mesh.example.com')
        hold = new Set(['mesh::status', 'configuration::get'])
        release()
        await expect.poll(() => held.length).toBe(2)
        let stored = { ...initialConfig, tunnel: { enabled: true } }
        assert.deepEqual(saved.at(-1).value, stored, 'configuration::set receives the stored entry with only the changed keys')
        await expect(tunnelSwitch).toBeChecked()
        await expect(badge).toHaveCount(0)
        await expect(textbox('Public URL')).toHaveValue('https://mesh.example.com')
        await expect(saveBar).toContainText('Unsaved changes · applied within a few seconds')
        hold = new Set()
        release()
        await button('Discard').click()
        await expect(saveBar).toHaveCount(0)
        await expect(badge).toHaveCount(0)
        await expect(tunnelLine).toHaveText('Starting the tunnel…')
        await capture('tunnel-starting')
        let before = statusCalls()
        tunnelPhase = { state: 'ready', url: tunnelURL, proxy_protocol: 'v2' }
        if (!timed) await button('Refresh').click()
        await expect(tunnelLine.getByRole('button', { name: 'Copy Tunnel URL' })).toBeVisible({ timeout: 8000 })
        if (timed) {
          assert(statusCalls() > before, 'mesh::status is re-read without a click while the tunnel settles')
          before = statusCalls()
          await delay(6000)
          assert.equal(statusCalls(), before, 'polling stops once the tunnel is ready')
        }
        await expect(tunnelLine.locator('code')).toHaveText(tunnelURL)
        // The line is an item of the settings list and announces what polling finds.
        await expect(tunnelLine).toHaveAttribute('aria-live', 'polite')
        await expect(page.getByRole('region', { name: 'Hub address', exact: true }).locator('[role="list"] > :not([role="listitem"])')).toHaveCount(0)
        await tunnelLine.getByRole('button', { name: 'Copy Tunnel URL' }).click()
        assert.equal(await page.evaluate(() => navigator.clipboard.readText()), tunnelURL)
        // With the tunnel alone, the Hub URL another engine pastes is the hub's effective one, not the empty Public URL.
        await button('Copy Join code').click()
        await expect(button('Copy Join code')).toHaveText('Copied')
        assert.equal(await page.evaluate(() => navigator.clipboard.readText()), joinCode)
        await button('Copy Hub URL').click()
        await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(tunnelURL)
        await capture('tunnel-ready')
        await refreshTo({ state: 'reconnecting' })
        await expect(tunnelLine).toHaveText('Starting the tunnel…')
        await refreshTo({ state: 'unavailable', error: 'quick-tunnel is not responding' })
        await expect(tunnelLine.getByText('The tunnel is unavailable', { exact: true })).toBeVisible()
        await expect(tunnelLine.getByText('quick-tunnel is not responding', { exact: true })).toBeVisible()
        await capture('tunnel-unavailable')
        await refreshTo({ state: 'failed', error: 'opentunnel relay unreachable' })
        await expect(tunnelLine.getByText('The tunnel failed', { exact: true })).toBeVisible()
        await expect(tunnelLine.getByText('opentunnel relay unreachable', { exact: true })).toBeVisible()
        await expect(tunnelLine.getByText('The tunnel is unavailable', { exact: true })).toHaveCount(0)
        if (timed) {
          before = statusCalls()
          await delay(6000)
          assert.equal(statusCalls(), before, 'polling stops once the tunnel failed')
        }
        await refreshTo({ state: 'stopped' })
        await expect(tunnelLine).toHaveCount(0)
        await expect(tunnelSwitch).toBeChecked()
        if (timed) {
          before = statusCalls()
          await expect.poll(statusCalls, { timeout: 8000 }).toBeGreaterThan(before)
        }
        await textbox('Public URL').fill('https://mesh.example.com')
        await expect(saveBar).toContainText('Unsaved changes · applied within a few seconds')
        stored = await savePage({ ...stored, public_url: 'https://mesh.example.com' })
        await expect(page.getByRole('heading', { name: 'Connect another engine', exact: true })).toBeVisible()
        await button('Copy Hub URL').click()
        await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('https://mesh.example.com')
        await capture('connect')
        await expect(tunnelLine).toHaveText('Not used while a Public URL is set')
        await capture('tunnel-overridden')
        // The hub drops the tunnel only when it applies the entry; the page keeps reading until it does.
        if (timed) applied = config
        await tunnelSwitch.click()
        stored = await savePage({ ...stored, tunnel: { enabled: false } })
        if (timed) {
          await expect(tunnelLine).toHaveText('Not used while a Public URL is set')
          before = statusCalls()
          applied = null
          await expect(tunnelLine).toHaveCount(0, { timeout: 8000 })
          assert(statusCalls() > before, 'mesh::status is re-read without a click until the hub drops the tunnel')
        }
        await expect(tunnelLine).toHaveCount(0)
        await expect(badge).toHaveCount(0)
        // A template that expands to nothing leaves the hub without an address: the page follows what the hub reports.
        config = { ...stored, public_url: '${MESH_PUBLIC_URL:}' }
        await button('Refresh').click()
        await expect(textbox('Public URL')).toHaveValue('${MESH_PUBLIC_URL:}')
        await expect(badge).toBeVisible()
        config = { ...config, tunnel: { enabled: true } }
        tunnelPhase = { state: 'starting' }
        await button('Refresh').click()
        await expect(tunnelLine).toHaveText('Starting the tunnel…')
        await expect(badge).toHaveCount(0)
        tunnelPhase = { state: 'ready', url: tunnelURL }
        if (!timed) await button('Refresh').click()
        await expect(tunnelLine.locator('code')).toHaveText(tunnelURL, { timeout: 8000 })
        // The Hub URL is what the hub expanded, never the template.
        await button('Copy Hub URL').click()
        await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(tunnelURL)
        // A raw root belongs to the configuration form: the page never writes its basics over it.
        config = structuredClone(stored)
        await button('Refresh').click()
        await expect(tunnelSwitch).not.toBeChecked()
        await tunnelSwitch.click()
        config = '${MESH_CONFIG}'
        const writes = saved.length
        await button('Save').click()
        await expect(page.getByText('Could not save', { exact: true })).toBeVisible()
        await expect(page.getByText('The mesh settings are one raw value', { exact: true })).toBeVisible()
        assert.equal(saved.length, writes, 'a raw root is never overwritten by the page')
        await expect(tunnelSwitch).toHaveCount(0)
        await expect(page.getByRole('radiogroup', { name: 'Mesh mode' })).toHaveCount(0)
        await expect(saveBar).toHaveCount(0)
        await capture('raw-root')
        config = structuredClone(stored)
        await button('Refresh').click()
        await button('Discard').click()
        await expect(tunnelSwitch).not.toBeChecked()
        console.log(`PASS ${viewport.width}px ${theme}: hub basics, discard, save writes only changed keys and keeps edits typed meanwhile, tunnel switch, every tunnel state, templated address, raw root refused${timed ? ', polling until settled' : ''}`)

        // The configuration form holds only the advanced settings.
        await button('Settings').click()
        await expect(page).toHaveURL(/#\/configuration\/workers\/mesh-lab$/)
        await expect(page.getByText('Mode, hub address, join code and shared workers are set on the Mesh page. These are the advanced settings.')).toBeVisible()
        for (const label of ['Listen address', 'Gateway port', 'Proxy URL', 'STUN port']) await expect(textbox(label)).toBeVisible()
        await expect(textbox('Public URL')).toHaveCount(0)
        await expect(textbox('Hub URL')).toHaveCount(0)
        await expect(page.getByRole('radiogroup', { name: 'Mesh mode' })).toHaveCount(0)
        await expect(tunnelSwitch).toHaveCount(0)
        const proxyUrl = textbox('Proxy URL')
        await proxyUrl.fill('ws://127.0.0.1:49201')
        await expect(button('Save changes')).toBeEnabled()
        await expect(button('Reset')).toBeVisible()
        await button('Reset').click()
        await expect(proxyUrl).toHaveValue(initialConfig.proxy_url)
        await textbox('Gateway port').fill('0')
        await expect(button('Save changes')).toBeDisabled()
        await textbox('Gateway port').fill('49135')
        await page.getByText('Advanced networking', { exact: true }).click()
        await page.getByRole('region', { name: 'Trusted proxies', exact: true }).getByRole('button', { name: 'Add item' }).click()
        await textbox('Trusted proxies 1').fill('127.0.0.1/32')
        for (const retired of ['Machine workers', 'Shared drives', 'Machine links']) await expect(page.getByText(retired, { exact: true })).toHaveCount(0)
        await capture('configuration')
        const count = saved.length
        await button('Save changes').click()
        await expect.poll(() => saved.length).toBe(count + 1)
        await expect(button('Save changes')).toHaveCount(0)
        assert.equal(saved.at(-1).value.listen, initialConfig.listen, 'unrelated environment template preserved')
        assert.equal(saved.at(-1).value.public_url, stored.public_url, 'page basics survive a form save')
        assert.deepEqual(saved.at(-1).value.tunnel, stored.tunnel)
        assert.deepEqual(saved.at(-1).value.trusted_proxies, ['127.0.0.1/32'])
        assert.deepEqual([saved.at(-1).value.links, saved.at(-1).value.nodes, saved.at(-1).value.drives], [[], {}, {}], 'retired keys pass through a form save')
        stored = saved.at(-1).value
        console.log(`PASS ${viewport.width}px ${theme}: advanced-only form, host validation/save/reset, templates, trusted proxy edit, retired keys kept`)

        // Client mode: hub fields go away, client fields and the share list appear, and a restart applies it.
        // The page is not re-read on return, so its Save must not write back the entry it loaded before the form saved.
        await openPage()
        await page.getByRole('radio', { name: 'Client', exact: true }).click()
        await expect(page.getByText('This project joins another project\'s hub and shares the workers you pick below.')).toBeVisible()
        await expect(page.getByRole('heading', { name: 'Hub address' })).toHaveCount(0)
        await expect(tunnelSwitch).toHaveCount(0)
        await expect(textbox('Public URL')).toHaveCount(0)
        await expect(page.getByRole('heading', { name: 'Machines', exact: true })).toHaveCount(0)
        await expect(page.getByRole('heading', { name: 'Hub connection', exact: true })).toBeVisible()
        await expect(page.locator('[data-field="client.join"]')).toHaveCount(1)
        await expect(page.getByRole('heading', { name: 'Shared workers', exact: true })).toBeVisible()
        await expect(page.getByRole('switch', { name: 'Share shell', exact: true })).not.toBeChecked()
        await expect(page.getByRole('switch', { name: 'Share state', exact: true })).not.toBeChecked()
        for (const never of ['mesh', 'secrets']) await expect(page.getByRole('switch', { name: `Share ${never}` })).toHaveCount(0)
        await expect(saveBar).toContainText('Unsaved changes · applied after a restart')
        await textbox('Hub URL').fill('https://hub.example.com')
        await textbox('Machine name').fill('Bad Name')
        await expect(page.getByText('Use lowercase letters, digits or hyphens, starting with a letter.')).toBeVisible()
        await expect(button('Save')).toBeDisabled()
        await textbox('Machine name').fill('laptop')
        await expect(button('Save')).toBeEnabled()
        await page.getByRole('switch', { name: 'Share shell', exact: true }).click()
        await expect(page.getByText('shell can run commands on this machine', { exact: true })).toBeVisible()
        await capture('client')
        stored = await savePage({ ...stored, mode: 'client', client: { hub_url: 'https://hub.example.com', name: 'laptop', share: ['shell'] } })
        await expect(page.getByText('Restart the mesh worker to switch to client mode', { exact: true })).toBeVisible()
        await button('Restart mesh').click()
        await expect(page.getByText('Restart the mesh worker to switch to client mode')).toHaveCount(0)
        assert(calls.some((call) => call.id === 'compose::restart'), 'Restart mesh calls compose::restart')
        await expect(page.getByRole('region', { name: 'Hub connection' }).getByText('connected', { exact: true })).toBeVisible()
        await expect(page.getByText('joined as')).toContainText('laptop')
        await expect(page.getByText('shell::exec', { exact: true })).toBeVisible()
        await capture('client-connected')
        clientState = 'restart required'
        await button('Refresh').click()
        await expect(page.getByText('Restart the mesh worker to apply the new connection', { exact: true })).toBeVisible()
        // The host form keeps the entry it loaded before the page saved; a fresh document reads the stored one.
        await page.goto(`${url}/?client=${randomUUID()}#/worker/mesh/mesh-manager`)
        await button('Settings').click()
        await expect(page).toHaveURL(/#\/configuration\/workers\/mesh-lab$/)
        await expect(page.getByText('Client mode has no advanced settings.', { exact: true })).toBeVisible()
        for (const label of ['Listen address', 'Proxy URL', 'Public URL', 'Hub URL']) await expect(textbox(label)).toHaveCount(0)
        await expect(page.getByText('Advanced networking', { exact: true })).toHaveCount(0)
        assert(calls.some((call) => call.id === 'configuration::get' && call.data.raw === true && call.namespace === 'default'))
        assert(calls.every((call) => !call.id.startsWith('configuration::') || call.namespace === 'default'))
        assert.deepEqual(errors, [])
        console.log(`PASS ${viewport.width}px ${theme}: client fields, machine-name validation, share list, restart panel, client-only form`)
      }
      const openDraft = async (value) => {
        config = structuredClone(value)
        // A fresh document gives each draft its own host baseline/query cache.
        // An unconstrained fixture schema lets the host serialize pre-existing custom shapes.
        activeSchema = {}
        mode = 'populated'
        running = 'hub'
        await page.goto(`${url}/?draft=${randomUUID()}#/worker/mesh/mesh-manager`)
        try {
          await button('Settings').click()
        } catch (error) {
          await writeFile(resolve(artifacts, 'host-open-draft-diagnostic.json'), JSON.stringify({ errors, calls, body: await page.locator('body').innerText() }, null, 2))
          throw error
        }
        await expect(page).toHaveURL(/#\/configuration\/workers\/mesh-lab$/)
      }
      const persist = async (expected) => {
        const count = saved.length
        await button('Save changes').click()
        await expect.poll(() => saved.length).toBe(count + 1)
        await expect(button('Save changes')).toHaveCount(0)
        assert.deepEqual(saved.at(-1).value, expected, 'host receives the entire preserved draft')
      }
      const replace = (label) => button(`Replace ${label} raw value with a literal value`)
      const unchanged = async () => {
        await expect(button('Save changes')).toHaveCount(0)
      }
      // public_url moved to the page; the form's unrelated edit is now the Proxy URL.
      const otherProxy = 'ws://127.0.0.1:49201'
      if (!opaqueOnly) {
        const focusDraft = { ...initialConfig, trusted_proxies: ['127.0.0.1/32'] }
        for (const field of ['trusted_proxies.0']) {
          await openDraft(focusDraft)
          const target = page.locator(`[data-field="${field}"]`)
          await expect(target).toHaveCount(1)
          const disclosure = target.locator('xpath=ancestor::details[1]')
          await expect(disclosure).not.toHaveAttribute('open', '')
          await expect(target).toBeHidden()
          await page.evaluate((field) => { window.location.hash = `/configuration/workers/mesh-lab/${field.split('.').join('/')}` }, field)
          await expect(target).toBeVisible()
          await expect(target).toBeFocused()
          await expect(disclosure).toHaveAttribute('open', '')
          await unchanged()
          await capture(`focus-${field}`)
        }
        console.log(`PASS ${viewport.width}px ${theme}: deep links reveal and focus closed advanced settings`)
      }
      if (focusOnly) {
        await context.close()
        continue
      }
      if (!opaqueCase || opaqueCase === 'root') {
        await openDraft('${MESH_CONFIG}')
        await expect(textbox('Listen address')).toHaveCount(0)
        const rawRoot = textbox('Mesh configuration')
        await expect(rawRoot).toHaveValue('${MESH_CONFIG}')
        await unchanged()
        await capture('opaque-root')
        await rawRoot.fill('${OTHER_MESH_CONFIG}')
        await persist('${OTHER_MESH_CONFIG}')
        await openDraft('${MESH_CONFIG}')
        await replace('Mesh configuration').click()
        await textbox('Proxy URL').fill(otherProxy)
        await persist({ ...defaults, proxy_url: otherProxy })
        console.log(`PASS ${viewport.width}px ${theme}: opaque root remains raw until explicit conversion; complete raw/default drafts round-trip through host`)
      }
      if (!opaqueCase && viewport.width === 1440 && theme === 'light') {
        for (const value of [null, false, 42, ['${ROOT_ITEM}']]) {
          await openDraft(value)
          const raw = textbox('Mesh configuration')
          await expect(raw).toHaveValue(JSON.stringify(value))
          await expect(raw).toHaveJSProperty('readOnly', true)
          await expect(textbox('Listen address')).toHaveCount(0)
          await unchanged()
          if (value === 42) await capture('custom-root')
          await replace('Mesh configuration').click()
          await persist(defaults)
        }
        // The Public URL boundary left with the field: the form no longer renders public_url.
        const boundaries = [
          { name: 'Trusted proxies', advanced: true, patch: { trusted_proxies: null }, after: { trusted_proxies: [] } },
          { name: 'Proxy URL', patch: { proxy_url: { custom: '${PROXY}' } }, after: { proxy_url: '' } },
          { name: 'Gateway port', patch: { gateway_port: '${GATEWAY_PORT}' }, after: { gateway_port: 49135 } },
        ]
        for (const boundary of boundaries) {
          const draft = { ...initialConfig, ...boundary.patch, unrelated: '${KEEP_THIS}' }
          await openDraft(draft)
          if (boundary.advanced) await page.getByText('Advanced networking', { exact: true }).click()
          const raw = textbox(boundary.name)
          await expect(raw).toBeVisible()
          await unchanged()
          if (boundary.name !== 'Proxy URL') {
            await textbox('Proxy URL').fill(otherProxy)
            await persist({ ...draft, proxy_url: otherProxy })
            await openDraft(draft)
            if (boundary.advanced) await page.getByText('Advanced networking', { exact: true }).click()
          }
          await replace(boundary.name).click()
          await persist({ ...draft, ...boundary.after })
        }
        const absent = { public_url: '', listen: '${KEEP_LISTENER}' }
        await openDraft(absent)
        await unchanged()
        await textbox('Proxy URL').fill(otherProxy)
        await persist({ ...absent, proxy_url: otherProxy })
        console.log('PASS opaque shape matrix: 4 custom roots, 3 nested boundaries, absent optional parents, explicit conversion and complete sibling/template preservation')
      }
      if (timed && !opaqueOnly && !focusOnly) {
        // A hub that never applies the saved tunnel: polling stops after its limit and the page says so.
        // Last in this context, because the fake clock stays installed in it.
        config = { ...initialConfig, tunnel: { enabled: true } }
        applied = initialConfig
        await page.clock.install()
        await page.goto(`${url}/?stall=${randomUUID()}#/worker/mesh/mesh-manager`)
        await expect(tunnelSwitch).toBeChecked()
        let before = statusCalls()
        await expect.poll(statusCalls, { timeout: 8000 }).toBeGreaterThan(before)
        const stalled = page.getByText('The hub has not reported a tunnel result. Check the mesh and quick-tunnel workers, then refresh.', { exact: true })
        await expect(stalled).toHaveCount(0)
        await page.clock.fastForward(13 * 60_000)
        await expect(stalled).toBeVisible()
        before = statusCalls()
        await page.clock.fastForward(30_000)
        await delay(1000)
        assert.equal(statusCalls(), before, 'polling stops at its limit')
        applied = null
        console.log(`PASS ${viewport.width}px ${theme}: tunnel polling stops after its limit and says so`)
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
