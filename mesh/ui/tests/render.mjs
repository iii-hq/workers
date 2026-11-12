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
  const newCode = () => {
    const codeRaw = [...randomBytes(8)].map((n) => '0123456789ABCDEFGHJKMNPQRSTVWXYZ'[n % 32]).join('')
    return `${codeRaw.slice(0, 4)}-${codeRaw.slice(4)}`
  }
  // mesh::join::cycle replaces it, as the hub does.
  let joinCode = newCode()
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
      workers: [{ name: 'browser', state: 'shared', functions: ['gpu-box::browser::act', 'gpu-box::browser::chromium::install'] },
        { name: 'image-resize', state: 'shared', functions: ['gpu-box::image_resize::resize', 'gpu-box::image_resize::thumbnail'] }, { name: 'tmux', state: 'pending', functions: [] }] },
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
    { function_id: 'gpu-box::browser::act', worker_name: bridge('gpu-box'), description: `Interact with the page: click, hover, type, select, press, scroll or drag. ${'A ref is scrolled into view first. '.repeat(6)}` },
    longFunction,
  ]
  const longNodes = [{ ...nodes[2], name: 'engineering-workstation-archive', path: 'relay / ' + 'long-network-path/'.repeat(12), version: '0.1.0-build-' + '1234567890'.repeat(8),
    workers: [{ name: 'worker-with-a-very-long-name', state: 'waiting for the saved operator to sign in', functions: [longFunction.function_id] },
      { name: 'another-worker-with-a-long-name', state: 'shared', functions: [] }] }]
  // Local workers the share lists offer; mesh, secrets and approval-gate are never shareable.
  const localWorkers = ['shell', 'state', 'mesh', 'secrets', 'approval-gate']
  // A client registers what the mesh shares with it as its own functions, under the mesh worker.
  const mirrorFunctions = [
    { function_id: 'hub::shell::exec', worker_name: 'mesh', description: 'Runs a command' },
    { function_id: 'peer::gpu-box::image_resize::resize', worker_name: 'mesh', description: 'Resizes an image' },
  ]
  // What a connected client can call, by owner, hub first, as its mesh::status reports it.
  const meshRemote = [{ machine: 'hub', functions: ['hub::shell::exec', 'hub::shell::spawn'] },
    { machine: 'gpu-box', functions: ['peer::gpu-box::image_resize::resize', 'peer::gpu-box::image_resize::thumbnail'] }]
  const results = []
  for (const viewport of [{ width: 1440, height: 1000 }, { width: 390, height: 844 }]) {
    for (const theme of ['light', 'dark']) {
      // That nothing is polled is checked in real time once: the page moves only on what the worker pushes.
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
      // The machines this context's hub lists; mesh::nodes::remove drops one.
      let hubNodes = nodes
      // What mesh::leave answers when the hub could not be told.
      let leaveWarning = 'The hub was not told (context deadline exceeded). It keeps listing this machine until it is removed there.'
      // The entry the hub last applied; null follows every save at once. Unset ${VAR} templates expand to their default.
      let applied = null
      // shared and catalog_skipped as the hub reports them, over what follows the applied entry.
      let hubExtra = {}
      const hubStatus = () => {
        const hub = applied ?? config
        const publicURL = String(hub?.public_url ?? '').replace(/\$\{\w+:?([^}]*)\}/g, '$1')
        const tunnel = hub?.tunnel?.enabled !== true ? undefined
          : publicURL ? { enabled: true, tunnel_id: 'mesh', state: 'overridden' }
          : { enabled: true, tunnel_id: 'mesh', ...tunnelPhase }
        const shared = (Array.isArray(hub?.share) ? hub.share : []).map((worker) => ({ worker, state: 'shared',
          functions: worker === 'shell' ? ['hub::shell::exec', 'hub::shell::spawn'] : [] }))
        return { version: '0.1.0', public_url: publicURL || tunnel?.url || '', ...(tunnel ? { tunnel } : {}), shared, catalog_skipped: [], ...hubExtra }
      }
      // Calls named in `hold` wait in `held` until release() answers them.
      let hold = new Set()
      const held = []
      // The page's latest mesh::on-node-changed and mesh::on-status-changed observes, to deliver events as the worker would.
      let nodeObserver = null
      let statusObserver = null
      const release = () => held.splice(0).forEach((answer) => answer())
      // What the client worker has applied; null follows the saved entry at once. A worker lags a save until it handles the event.
      let appliedShare = null
      let hubName = null
      let requestedName = null
      let joinedHub = null
      // error, retrying and rename_error, as the worker reports them.
      let clientExtra = {}
      // What a joined client can call; a client that is not joined holds nothing.
      let clientRemote = meshRemote
      // Only a joined node has a name; a removed one says how to come back, and a left one keeps the leave's warning.
      const clientStatus = () => {
        const joined = ['connected', 'renaming', 'restart required'].includes(clientState)
        const node = joined ? hubName ?? config.client?.name ?? '' : ''
        return { mode: 'client', hub_url: joinedHub ?? config.client?.hub_url ?? '', namespace: 'project', node, state: clientState,
          ...(joined ? { requested_name: requestedName ?? config.client?.name ?? '' } : {}),
          error: clientState === 'removed by hub' ? 'the hub removed this machine; leave the hub, then join with a new join code' : clientState === 'left hub' ? leaveWarning : '',
          shared: (appliedShare ?? config.client?.share ?? []).map((worker) => ({ worker, state: joined ? 'shared' : 'pending',
            functions: worker === 'shell' ? ['shell::exec', 'shell::spawn'].map((id) => node ? `${node}::${id}` : id) : [] })),
          remote: joined ? clientRemote : [], ...clientExtra }
      }
      // One call of mesh::on-status-changed, delivered to the page's observer: the worker's current mesh::status.
      const pushStatus = () => {
        assert(statusObserver, 'the page observes mesh::on-status-changed')
        statusObserver.socket.send(JSON.stringify({ type: 'invokefunction', invocation_id: randomUUID(), function_id: statusObserver.frame.target.function_id,
          activation: { observer_id: statusObserver.frame.id }, data: running === 'client' ? clientStatus() : hubStatus() }))
      }
      await page.routeWebSocket('**/ws', (socket) => {
        socket.onMessage(async (message) => {
          const frame = JSON.parse(String(message))
          if (frame.type === 'invokefunction' && frame.function_id === 'engine::workers::register') {
            socket.send(JSON.stringify({ type: 'workerregistered', worker_id: randomUUID(), features: ['observe.v1', 'observe.v2', 'local-dispatch.v1'] }))
          }
          if (frame.type === 'registerfunction') socket.send(JSON.stringify({ type: 'functionregistered', id: frame.id, namespace: frame.namespace ?? 'project', revision: 0, local_dispatch: false, observers: [] }))
          if (frame.type === 'observe') socket.send(JSON.stringify({ type: 'observeresult', id: frame.id, state: 'active' }))
          if (frame.type === 'observe' && frame.source.function_id === 'mesh::on-node-changed') nodeObserver = { socket, frame }
          if (frame.type === 'observe' && frame.source.function_id === 'mesh::on-status-changed') statusObserver = { socket, frame }
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
              case 'mesh::nodes::list': return mode === 'error' ? send(null, 'Mesh hub is unavailable. Reconnect, then retry.') : send(mode === 'empty' || mode === 'loading' ? [] : mode === 'long' ? longNodes : hubNodes)
              case 'mesh::nodes::remove':
                if (mode === 'remove-error') return send(null, 'The hub could not save its state.')
                assert(hubNodes.some((node) => node.name === frame.data.name), 'mesh::nodes::remove names a listed machine')
                hubNodes = hubNodes.filter((node) => node.name !== frame.data.name)
                return send({ removed: true, code: (joinCode = newCode()) })
              case 'mesh::leave':
                if (mode === 'leave-error') return send(null, 'not joined to a hub')
                clientState = 'left hub'
                return send({ hub_notified: !leaveWarning, warning: leaveWarning })
              // Like the hub, no join code while there is no public URL (configured or from a ready tunnel).
              case 'mesh::join::code': return mode === 'error' || !hubStatus().public_url ? send(null, 'public_url is not set') : send({ code: joinCode })
              case 'mesh::join::cycle': return mode === 'cycle-error' ? send(null, 'The hub could not store a new code.') : !hubStatus().public_url ? send(null, 'public_url is not set') : send({ code: (joinCode = newCode()) })
              case 'mesh::status': return send(running === 'client' ? clientStatus() : hubStatus())
              case 'compose::restart': assert.deepEqual(frame.data, { container: 'mesh' }); running = config?.mode === 'client' ? 'client' : 'hub'; return send({})
              case 'configuration::list': return send({ configurations: [{ ...entry, schema: activeSchema }] })
              case 'configuration::schema': assert.equal(frame.data.id, 'mesh-lab'); return send({ ...entry, schema: activeSchema })
              case 'configuration::get': assert.equal(frame.data.id, 'mesh-lab'); return send({ id: 'mesh-lab', value: config })
              case 'configuration::set': assert.equal(frame.data.id, 'mesh-lab'); saved.push(frame.data); config = frame.data.value; return send({ new_value: config, old_value: initialConfig })
              case 'engine::functions::list': return mode === 'functions-error' ? send(null, 'Engine function list is unavailable.') : send({ functions: [...localWorkers.map((worker) => ({ function_id: `${worker}::fixture`, worker_name: worker })), ...hubFunctions, ...mirrorFunctions] })
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
      const dialog = page.getByRole('alertdialog')
      const inDialog = (name) => dialog.getByRole('button', { name, exact: true })
      const shareRow = (name) => page.locator('.mesh-ui-share > li').filter({ has: page.getByRole('switch', { name: `Share ${name}`, exact: true }) })
      // Page Save: only the edited basics change; every other key, templates included, is written back as read.
      const savePage = async (expected) => {
        const count = saved.length
        await button('Save').click()
        await expect.poll(() => saved.length).toBe(count + 1)
        await expect(saveBar).toHaveCount(0)
        assert.deepEqual(saved.at(-1).value, expected, 'configuration::set receives the stored entry with only the changed keys')
        return expected
      }
      // The hub pushes each tunnel change; nothing is clicked.
      const pushTo = (phase) => {
        tunnelPhase = phase
        pushStatus()
      }
      // In real time, once: with nothing pushed, the page reads mesh::status no more.
      const idle = async (what) => {
        if (!timed) return
        const before = statusCalls()
        await delay(6000)
        assert.equal(statusCalls(), before, `${what} is not polled`)
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
        // Each machine shows what it shares: one row per worker with its function count, the functions behind it.
        const open = (scope, worker) => scope.locator('summary').filter({ has: page.getByText(worker, { exact: true }) }).click()
        await expect(workstation.getByText('shared · 2 functions', { exact: true })).toBeVisible()
        await expect(workstation.getByRole('list', { name: 'ide functions', exact: true })).toBeHidden()
        await open(workstation, 'ide')
        // The machine prefix is said once; ide's ids do not share a worker prefix, so coder:: and shell:: stay.
        await expect(workstation.getByText('Ids start with linux-workstation::', { exact: true })).toBeVisible()
        await expect(workstation.getByRole('list', { name: 'ide functions', exact: true }).getByRole('listitem')).toHaveText([
          'coder::read-file Reads a file from the workspace', 'shell::exec',
        ])
        await expect(workstation.getByText('Reads a file from the workspace', { exact: true })).toBeVisible()
        const gpu = card('gpu-box')
        await open(gpu, 'browser')
        await open(gpu, 'image-resize')
        await expect(gpu.getByText('Ids start with gpu-box::browser::', { exact: true })).toBeVisible()
        const act = gpu.getByRole('list', { name: 'browser functions', exact: true }).getByRole('listitem')
        await expect(act).toHaveText([/^act Interact with the page/, 'chromium::install'])
        // A long description stays on one line.
        const [line, text] = await act.first().locator('span').evaluate((el) => [parseFloat(getComputedStyle(el).lineHeight) || 20, el.getBoundingClientRect().height])
        assert(text <= line * 1.5, `a function description takes one line (${text}px)`)
        await expect(gpu.getByRole('list', { name: 'image-resize functions', exact: true }).getByRole('listitem')).toHaveText([
          'image_resize::resize Resizes an image', 'image_resize::thumbnail',
        ])
        await expect(gpu.getByRole('list', { name: 'tmux functions', exact: true })).toHaveCount(0)
        await expect(gpu.getByRole('listitem').filter({ hasText: 'tmux' }).getByText('pending · No functions registered yet', { exact: true })).toBeVisible()
        await expect(card('archive-laptop').getByText('Not sharing any workers', { exact: true })).toBeVisible()
        const travel = card('travel-laptop')
        await expect(travel.getByRole('listitem').filter({ hasText: 'mesh-probe' }).getByText('Unavailable while the machine is offline', { exact: true })).toBeVisible()
        await expect(travel.getByText('travel-laptop::mesh-probe::echo')).toHaveCount(0)
        // The hub keeps an offline machine's last state; showing "shared" would contradict the line above.
        await expect(travel.getByText('shared', { exact: true })).toHaveCount(0)
        const headings = await page.getByRole('heading', { level: 2 }).allTextContents()
        assert(headings.includes('Machines') && headings.indexOf('Machines') < headings.indexOf('Shared workers') && headings.indexOf('Shared workers') < headings.indexOf('Hub address'),
          'Machines, then Shared workers, then Hub address in hub mode')
        await capture('populated')
        // The hub has no public URL yet: no join code, and the machines stay listed.
        await expect(page.getByText('Join instructions unavailable')).toBeVisible()
        await expect(button('Copy Join code')).toHaveCount(0)
        await expect(button('Copy Hub URL')).toHaveCount(0)
        await expect(button('Generate new code')).toHaveCount(0)
        mode = 'long'; await button('Refresh').click()
        await expect(page.getByRole('heading', { name: longNodes[0].name })).toBeVisible()
        await open(page, 'worker-with-a-very-long-name')
        await expect(page.getByRole('list', { name: 'worker-with-a-very-long-name functions', exact: true }).getByRole('listitem')).toHaveText([`${longFunction.function_id.slice(longNodes[0].name.length + 2)} ${longFunction.description}`])
        await capture('long')
        // A failed function list loses only the descriptions: the hub's ids still show.
        mode = 'functions-error'; await button('Refresh').click()
        await open(workstation, 'ide')
        await expect(workstation.getByRole('list', { name: 'ide functions', exact: true }).getByRole('listitem')).toHaveText(['coder::read-file', 'shell::exec'])
        await expect(page.getByText(/Could not load functions/)).toHaveCount(0)
        // A refresh keeps an open worker open.
        mode = 'populated'; await button('Refresh').click()
        await expect(workstation.getByRole('list', { name: 'ide functions', exact: true })).toBeVisible()
        console.log(`PASS ${viewport.width}px ${theme}: loader, machine states, overflow, join refused without a public URL`)

        // Hub mode: the basics are on the page, client fields are not.
        mode = 'populated'
        await expect(page.getByRole('radio', { name: 'Hub', exact: true })).toBeChecked()
        await expect(page.getByText('This project is the hub. Other projects join it as clients; the hub and every machine can call the workers the others share.')).toBeVisible()
        await expect(page.getByRole('heading', { name: 'Hub address', exact: true })).toBeVisible()
        await expect(badge).toBeVisible()
        await expect(tunnelSwitch).not.toBeChecked()
        await expect(textbox('Public URL')).toHaveValue('')
        await expect(tunnelLine).toHaveCount(0)
        await expect(textbox('Hub URL')).toHaveCount(0)
        await expect(page.getByRole('heading', { name: 'Available from the mesh' })).toHaveCount(0)
        // The hub shares its own workers: the ones running on its engine, never the mesh's own or approval-gate.
        const hubShares = page.getByRole('region', { name: 'Shared workers', exact: true })
        await expect(hubShares.getByText('Nothing shared · joined machines call them as hub::function', { exact: true })).toBeVisible()
        await expect(hubShares.getByRole('switch')).toHaveCount(2)
        for (const name of ['shell', 'state']) await expect(hubShares.getByRole('switch', { name: `Share ${name}`, exact: true })).not.toBeChecked()
        await expect(hubShares.getByText('Every machine joined to this hub can call what you share.', { exact: true })).toBeVisible()
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
        await idle('a tunnel that is still starting')
        assert.equal(statusObserver?.frame.source.args, undefined, 'mesh::on-status-changed takes no args')
        let before = statusCalls()
        pushTo({ state: 'ready', url: tunnelURL, proxy_protocol: 'v2' })
        await expect(tunnelLine.getByRole('button', { name: 'Copy Tunnel URL' })).toBeVisible()
        assert(statusCalls() > before, 'the pushed status change makes the page read mesh::status again, without a click')
        await expect(tunnelLine.locator('code')).toHaveText(tunnelURL)
        // The line is an item of the settings list and announces what the hub reports.
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
        pushTo({ state: 'reconnecting' })
        await expect(tunnelLine).toHaveText('Starting the tunnel…')
        pushTo({ state: 'unavailable', error: 'quick-tunnel is not responding' })
        await expect(tunnelLine.getByText('The tunnel is unavailable', { exact: true })).toBeVisible()
        await expect(tunnelLine.getByText('quick-tunnel is not responding', { exact: true })).toBeVisible()
        await capture('tunnel-unavailable')
        pushTo({ state: 'failed', error: 'opentunnel relay unreachable' })
        await expect(tunnelLine.getByText('The tunnel failed', { exact: true })).toBeVisible()
        await expect(tunnelLine.getByText('opentunnel relay unreachable', { exact: true })).toBeVisible()
        await expect(tunnelLine.getByText('The tunnel is unavailable', { exact: true })).toHaveCount(0)
        pushTo({ state: 'stopped' })
        await expect(tunnelLine).toHaveCount(0)
        await expect(tunnelSwitch).toBeChecked()
        await textbox('Public URL').fill('https://mesh.example.com')
        await expect(saveBar).toContainText('Unsaved changes · applied within a few seconds')
        stored = await savePage({ ...stored, public_url: 'https://mesh.example.com' })
        await expect(page.getByRole('heading', { name: 'Connect another engine', exact: true })).toBeVisible()
        await expect(page.getByText(/A machine that joins can call every worker shared by this hub and by the other machines\./)).toBeVisible()
        await button('Copy Hub URL').click()
        await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('https://mesh.example.com')
        await capture('connect')
        // A new code replaces the old one before the page reads again, and is announced.
        const joinValue = page.locator('.mesh-ui-copy').filter({ has: button('Copy Join code') }).locator('code')
        const oldCode = joinCode
        await expect(joinValue).toHaveText(oldCode)
        hold = new Set(['mesh::join::cycle'])
        await button('Generate new code').click()
        await expect(button('Generating…')).toBeDisabled()
        hold = new Set(['mesh::join::code'])
        release()
        assert.notEqual(joinCode, oldCode, 'mesh::join::cycle returns a new code')
        await expect(joinValue).toHaveText(joinCode)
        await expect(page.getByText('New join code ready. The old code no longer works.', { exact: true })).toBeAttached()
        hold = new Set()
        release()
        await expect(button('Generate new code')).toBeEnabled()
        await expect(joinValue).toHaveText(joinCode)
        // A failed cycle keeps the code and says why; the next one clears the error.
        const kept = joinCode
        mode = 'cycle-error'
        await button('Generate new code').click()
        await expect(page.getByText('Could not generate a new code', { exact: true })).toBeVisible()
        await expect(page.getByText(/The hub could not store a new code\./)).toBeVisible()
        await expect(joinValue).toHaveText(kept)
        await capture('cycle-error')
        mode = 'populated'
        await button('Generate new code').click()
        await expect(page.getByText('Could not generate a new code', { exact: true })).toHaveCount(0)
        await expect(joinValue).toHaveText(joinCode)
        assert.notEqual(joinCode, kept, 'the next cycle returns a new code')
        // A machine that joins with the shown code spends it: the hub's node event makes the page read the replacement.
        assert.deepEqual(nodeObserver?.frame.source.args, {}, 'the hub page observes every node change')
        const spent = joinCode
        joinCode = newCode()
        nodeObserver.socket.send(JSON.stringify({ type: 'invokefunction', invocation_id: randomUUID(), function_id: nodeObserver.frame.target.function_id,
          activation: { observer_id: nodeObserver.frame.id }, data: { name: 'new-box', event: 'joined', at: new Date().toISOString() } }))
        await expect(joinValue).toHaveText(joinCode)
        assert.notEqual(joinCode, spent)
        // Remove asks first, says what it does, and a cancel calls nothing.
        const removeButton = card('archive-laptop').getByRole('button', { name: 'Remove archive-laptop', exact: true })
        await removeButton.click()
        await expect(dialog.getByRole('heading', { name: 'Remove archive-laptop?', exact: true })).toBeVisible()
        await expect(dialog).toContainText('cannot reconnect with its current keys')
        await expect(dialog).toContainText('loses access to everything shared')
        await expect(dialog).toContainText('The join code is replaced')
        await capture('remove-confirm')
        await inDialog('Cancel').click()
        await expect(dialog).toHaveCount(0)
        assert(!calls.some((call) => call.id === 'mesh::nodes::remove'), 'a cancelled removal calls nothing')
        // A failed removal stays in the dialog with its reason; the retry removes the machine and shows the hub's new code.
        mode = 'remove-error'
        await removeButton.click()
        await inDialog('Remove machine').click()
        await expect(dialog.getByRole('alert')).toContainText('The hub could not save its state.')
        await expect(page.locator('.mesh-ui-node')).toHaveCount(4)
        mode = 'populated'
        hold = new Set(['mesh::nodes::remove'])
        await inDialog('Retry remove machine').click()
        await expect(inDialog('Removing…')).toBeDisabled()
        const beforeRemoval = joinCode
        hold = new Set()
        release()
        await expect(dialog).toHaveCount(0)
        await expect(card('archive-laptop')).toHaveCount(0)
        await expect(page.locator('.mesh-ui-node h3')).toHaveText(['gpu-box', 'linux-workstation', 'travel-laptop'])
        assert.deepEqual(calls.findLast((call) => call.id === 'mesh::nodes::remove').data, { name: 'archive-laptop' })
        assert.notEqual(joinCode, beforeRemoval, 'a removal replaces the join code')
        await expect(joinValue).toHaveText(joinCode)
        await expect(page.getByText('Removed archive-laptop. The join code was replaced.', { exact: true })).toBeAttached()
        await expect(tunnelLine).toHaveText('Not used while a Public URL is set')
        await capture('tunnel-overridden')
        // The hub drops the tunnel only when it applies the entry, and says so then.
        applied = config
        await tunnelSwitch.click()
        stored = await savePage({ ...stored, tunnel: { enabled: false } })
        await expect(tunnelLine).toHaveText('Not used while a Public URL is set')
        applied = null
        pushStatus()
        await expect(tunnelLine).toHaveCount(0)
        await expect(badge).toHaveCount(0)
        // A hub share writes the top-level share, warns about a command runner, and follows what the hub pushes.
        await hubShares.getByRole('switch', { name: 'Share shell', exact: true }).click()
        await expect(hubShares.getByText('shell can run commands on this hub', { exact: true })).toBeVisible()
        await expect(hubShares.getByText('Every machine joined to this hub can call shared workers, and nothing asks here first, so sharing these gives each of them remote code execution with this project\'s access.', { exact: true })).toBeVisible()
        await expect(hubShares.getByText('Every machine joined to this hub can call what you share.')).toHaveCount(0)
        await expect(shareRow('shell').getByText('not saved yet', { exact: true })).toBeVisible()
        applied = config
        stored = await savePage({ ...stored, share: ['shell'] })
        await expect(shareRow('shell').getByText('applying…', { exact: true })).toBeVisible()
        applied = null
        pushStatus()
        await expect(shareRow('shell').getByText('shared', { exact: true })).toBeVisible()
        await expect(hubShares.getByText('1 shared · joined machines call them as hub::function', { exact: true })).toBeVisible()
        await shareRow('shell').locator('summary').click()
        await expect(hubShares.getByText('Ids start with hub::shell::', { exact: true })).toBeVisible()
        await expect(hubShares.getByRole('list', { name: 'shell functions', exact: true }).getByRole('listitem')).toHaveText(['exec', 'spawn'])
        await capture('hub-shared')
        // The states the hub cannot share in, each with what to do, and a machine left out of the catalog on its own card.
        hubExtra = { shared: [{ worker: 'shell', state: 'name conflict', functions: [] }], catalog_skipped: ['gpu-box'] }
        pushStatus()
        await expect(shareRow('shell').getByText('name conflict', { exact: true })).toBeVisible()
        await expect(shareRow('shell').getByText('A machine has the name these functions start with. Rename or remove it to share this worker.', { exact: true })).toBeVisible()
        await expect(card('gpu-box').getByText('gpu-box shares more than the mesh carries; other machines cannot call it.', { exact: true })).toBeVisible()
        await expect(page.getByText(/shares more than the mesh carries/)).toHaveCount(1)
        await capture('hub-share-conflict')
        hubExtra = { shared: [{ worker: 'shell', state: 'too large', functions: [] }] }
        pushStatus()
        await expect(shareRow('shell').getByText('Too many functions to share.', { exact: true })).toBeVisible()
        await expect(shareRow('shell').getByText(/A machine has the name/)).toHaveCount(0)
        await expect(page.getByText(/shares more than the mesh carries/)).toHaveCount(0)
        hubExtra = {}
        pushStatus()
        await expect(shareRow('shell').getByText('shared', { exact: true })).toBeVisible()
        await expect(shareRow('shell').getByText('Too many functions to share.')).toHaveCount(0)
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
        pushTo({ state: 'ready', url: tunnelURL })
        await expect(tunnelLine.locator('code')).toHaveText(tunnelURL)
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
        console.log(`PASS ${viewport.width}px ${theme}: hub basics, discard, save writes only changed keys and keeps edits typed meanwhile, tunnel switch, every tunnel state pushed by the hub, new join code shown and announced, failed cycle keeps the code, a join shows the replacement code, a machine is removed after a confirmation, hub shares saved, applying and pushed with their warning, conflict, size and skipped-machine lines, templated address, raw root refused${timed ? ', nothing polled' : ''}`)

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
        await expect(page.getByText('This project joins another project\'s hub. It shares the workers you pick below and can call what the hub and other machines share.')).toBeVisible()
        await expect(page.getByRole('heading', { name: 'Hub address' })).toHaveCount(0)
        await expect(tunnelSwitch).toHaveCount(0)
        await expect(textbox('Public URL')).toHaveCount(0)
        await expect(page.getByRole('heading', { name: 'Machines', exact: true })).toHaveCount(0)
        await expect(page.getByRole('heading', { name: 'Hub connection', exact: true })).toBeVisible()
        await expect(page.locator('[data-field="client.join"]')).toHaveCount(1)
        await expect(page.getByRole('heading', { name: 'Shared workers', exact: true })).toBeVisible()
        await expect(page.getByRole('switch', { name: 'Share shell', exact: true })).not.toBeChecked()
        await expect(page.getByRole('switch', { name: 'Share state', exact: true })).not.toBeChecked()
        for (const never of ['mesh', 'secrets', 'approval-gate']) await expect(page.getByRole('switch', { name: `Share ${never}` })).toHaveCount(0)
        await expect(page.getByText('The hub project and every machine joined to the hub can call what you share.', { exact: true })).toBeVisible()
        // Nothing runs as a client yet, so nothing is available from the mesh.
        const remoteList = page.getByRole('region', { name: 'Available from the mesh', exact: true })
        await expect(remoteList.getByText('Shown once this machine is connected', { exact: true })).toBeVisible()
        await expect(saveBar).toContainText('Unsaved changes · applied after a restart')
        await textbox('Hub URL').fill('https://hub.example.com')
        // Refused as well: names the hub would change (a hyphen at the end or next to another).
        for (const bad of ['Bad Name', 'work-', 'a--b']) {
          await textbox('Machine name').fill(bad)
          await expect(page.getByText('Use lowercase letters, digits and single hyphens, starting with a letter and not ending with a hyphen.')).toBeVisible()
          await expect(button('Save')).toBeDisabled()
        }
        await textbox('Machine name').fill('laptop')
        await expect(button('Save')).toBeEnabled()
        await page.getByRole('switch', { name: 'Share shell', exact: true }).click()
        await expect(page.getByText('shell can run commands on this machine', { exact: true })).toBeVisible()
        await expect(page.getByText('The hub project and every machine joined to the hub can call shared workers, and nothing asks here first, so sharing these gives all of them remote code execution here.', { exact: true })).toBeVisible()
        // "not saved yet" means only that the form differs from the stored entry.
        await expect(shareRow('shell').getByText('not saved yet', { exact: true })).toBeVisible()
        await capture('client')
        stored = await savePage({ ...stored, mode: 'client', client: { hub_url: 'https://hub.example.com', name: 'laptop', share: ['shell'] } })
        await expect(page.getByText('Restart the mesh worker to switch to client mode', { exact: true })).toBeVisible()
        // Saved, and the worker still runs as a hub: the share waits for the restart, it is not "not saved".
        await expect(shareRow('shell').getByText('starts after the restart', { exact: true })).toBeVisible()
        await expect(page.getByText('not saved yet')).toHaveCount(0)
        await button('Restart mesh').click()
        await expect(page.getByText('Restart the mesh worker to switch to client mode')).toHaveCount(0)
        assert(calls.some((call) => call.id === 'compose::restart'), 'Restart mesh calls compose::restart')
        await expect(page.getByRole('region', { name: 'Hub connection' }).getByText('connected', { exact: true })).toBeVisible()
        await expect(page.getByText('joined as')).toContainText('laptop')
        // The shared worker's functions sit behind a count, with the machine and worker prefix said once.
        await page.locator('.mesh-ui-share summary').filter({ hasText: '2 functions' }).click()
        await expect(page.getByText('Ids start with laptop::shell::', { exact: true })).toBeVisible()
        await expect(page.getByRole('list', { name: 'shell functions', exact: true }).getByRole('listitem')).toHaveText(['exec', 'spawn'])
        await expect(page.getByText('1 shared · the hub calls them as laptop::function, other machines as peer::laptop::function', { exact: true })).toBeVisible()
        // What the mesh shares with this machine, hub first, each owner behind a count, described from the local listing.
        await expect(remoteList.locator('summary')).toHaveText(['hub2 functions', 'gpu-box2 functions'])
        await remoteList.locator('summary').first().click()
        await expect(remoteList.getByText('Ids start with hub::', { exact: true })).toBeVisible()
        await expect(remoteList.getByRole('list', { name: 'hub functions', exact: true }).getByRole('listitem')).toHaveText(['shell::exec Runs a command', 'shell::spawn'])
        await remoteList.locator('summary').nth(1).click()
        await expect(remoteList.getByText('Ids start with peer::gpu-box::', { exact: true })).toBeVisible()
        await expect(remoteList.getByRole('list', { name: 'gpu-box functions', exact: true }).getByRole('listitem')).toHaveText(['image_resize::resize Resizes an image', 'image_resize::thumbnail'])
        await capture('client-connected')
        // The list follows what the worker pushes.
        clientRemote = []
        pushStatus()
        await expect(remoteList.getByText('Nothing is shared with this machine yet', { exact: true })).toBeVisible()
        clientRemote = meshRemote
        pushStatus()
        await expect(remoteList.locator('summary')).toHaveCount(2)
        // Leave hub asks first; a cancel calls nothing.
        const connection = page.getByRole('region', { name: 'Hub connection' })
        await button('Leave hub').click()
        await expect(dialog.getByRole('heading', { name: 'Leave the hub?', exact: true })).toBeVisible()
        await expect(dialog).toContainText('joins again only with a different join code or token')
        await expect(dialog).toContainText('loses access to everything shared')
        await capture('leave-confirm')
        await inDialog('Cancel').click()
        await expect(dialog).toHaveCount(0)
        assert(!calls.some((call) => call.id === 'mesh::leave'), 'a cancelled leave calls nothing')
        // A failed leave stays in the dialog; the retry leaves, and a hub that was not told is named.
        mode = 'leave-error'
        await button('Leave hub').click()
        await inDialog('Leave hub').click()
        await expect(dialog.getByRole('alert')).toContainText('not joined to a hub')
        mode = 'populated'
        hold = new Set(['mesh::leave'])
        await inDialog('Retry leave hub').click()
        await expect(inDialog('Leaving…')).toBeDisabled()
        hold = new Set()
        release()
        await expect(dialog).toHaveCount(0)
        await expect(connection.getByText('left hub', { exact: true })).toBeVisible()
        await expect(page.getByText('This engine left the hub', { exact: true })).toBeVisible()
        await expect(connection.getByText(leaveWarning, { exact: true })).toBeVisible()
        await expect(page.getByText(`Left the hub. ${leaveWarning}`, { exact: true })).toBeAttached()
        await expect(button('Leave hub')).toHaveCount(0)
        await expect(page.getByText('joined as')).toHaveCount(0)
        await expect(remoteList.getByText('Shown once this machine is connected', { exact: true })).toBeVisible()
        await capture('client-left')
        // The warning comes from the worker, so a re-read keeps it.
        await button('Refresh').click()
        await expect(connection.getByText(leaveWarning, { exact: true })).toBeVisible()
        await idle('a client that is not connected')

        // A join, as the worker pushes it: every step says what is happening, with no click and no timer.
        const panel = (headline) => connection.getByText(headline, { exact: true })
        const step = async (state, extra, headline, ...texts) => {
          clientState = state
          clientExtra = extra
          before = statusCalls()
          pushStatus()
          // The pushed status makes the page read mesh::status again, with no click.
          await expect.poll(statusCalls).toBeGreaterThan(before)
          await expect(panel(headline)).toBeVisible()
          if (state) await expect(connection.getByText(state, { exact: true })).toBeVisible()
          for (const text of texts) await expect(connection.getByText(text, { exact: false }).first()).toBeVisible()
        }
        await step('starting', {}, 'Starting…', 'The mesh worker is reading its settings.')
        await step('waiting for join', {}, 'Not joined yet', 'The worker joins as soon as they are stored.')
        await step('waiting for join', { error: 'resolve secret://MESH_JOIN: iii: invocation failed: function_not_found: secrets::resolve', retrying: true }, 'Not joined yet',
          'resolve secret://MESH_JOIN: iii: invocation failed: function_not_found: secrets::resolve', 'The worker keeps trying by itself.')
        await expect(connection.getByText('The worker joins as soon as they are stored.')).toHaveCount(0)
        await step('joining', {}, 'Joining the hub…', 'The worker is sending the join code to the hub.')
        await step('waiting for hub', {}, 'Waiting for the hub to accept this machine…', 'This takes up to a minute. Nothing to do.')
        await capture('client-waiting-for-hub')
        await step('error', { error: 'start mesh: hub did not accept the registration within 1m', retrying: true }, 'Not sharing',
          'start mesh: hub did not accept the registration within 1m', 'The worker keeps trying by itself.')
        await capture('client-error')
        await step('join failed', { error: 'hub refused the registration: invalid join code (change client.join to retry)' }, 'The hub refused the join',
          'hub refused the registration: invalid join code (change client.join to retry)', 'Store another join code or token. This one is not tried again.')
        await capture('client-join-failed')
        await step('connecting', {}, 'Connecting to the hub…', 'The worker is waiting for its name and gateway.')
        // Each step is announced to a screen reader.
        await expect(page.locator('.mesh-ui-progress')).toHaveAttribute('aria-live', 'polite')
        clientState = 'connected'
        clientExtra = {}
        pushStatus()
        await expect(connection.getByText('connected', { exact: true })).toBeVisible()
        await expect(page.locator('.mesh-ui-progress')).toBeEmpty()
        await expect(page.getByText('This engine left the hub')).toHaveCount(0)
        await idle('a connected client')
        clientExtra = { hub_wants_version: '0.2.0' }
        pushStatus()
        await expect(page.getByText('Nothing is shared or available until this project\'s mesh worker runs the hub\'s version.', { exact: true })).toBeVisible()
        clientExtra = {}
        pushStatus()
        await expect(page.getByText(/The hub wants version/)).toHaveCount(0)

        // A share is "not saved yet" only while the form is dirty; saved, it is being applied until the worker says otherwise.
        const stateSwitch = page.getByRole('switch', { name: 'Share state', exact: true })
        await expect(shareRow('shell').getByText('shared', { exact: true })).toBeVisible()
        await stateSwitch.click()
        await expect(shareRow('state').getByText('not saved yet', { exact: true })).toBeVisible()
        appliedShare = ['shell']
        stored = await savePage({ ...stored, client: { ...stored.client, share: ['shell', 'state'] } })
        await expect(shareRow('state').getByText('applying…', { exact: true })).toBeVisible()
        await expect(page.getByText('not saved yet')).toHaveCount(0)
        await capture('client-share-applying')
        appliedShare = null
        pushStatus()
        await expect(shareRow('state').getByText('shared', { exact: true })).toBeVisible()
        // Switched off: still shared until the save, then stopping until the worker dropped it.
        await stateSwitch.click()
        await expect(shareRow('state').getByText('shared until you save', { exact: true })).toBeVisible()
        appliedShare = ['shell', 'state']
        stored = await savePage({ ...stored, client: { ...stored.client, share: ['shell'] } })
        await expect(shareRow('state').getByText('stopping…', { exact: true })).toBeVisible()
        appliedShare = null
        pushStatus()
        await expect(shareRow('state').locator('.mesh-ui-muted')).toHaveCount(0)

        // A new machine name renames the joined machine, with no restart.
        hubName = 'laptop'
        requestedName = 'laptop'
        await textbox('Machine name').fill('work')
        await expect(saveBar).toContainText('Unsaved changes · applied within a few seconds')
        stored = await savePage({ ...stored, client: { ...stored.client, name: 'work' } })
        await expect(page.getByText(/Restart the mesh worker/)).toHaveCount(0)
        // Read before the worker handled the save: the rename is on its way, the old name still works.
        await expect(panel('Renaming this machine…')).toBeVisible()
        await expect(page.getByText('joined as')).toContainText('laptop')
        clientState = 'renaming'
        pushStatus()
        await expect(connection.getByText('renaming', { exact: true })).toBeVisible()
        await expect(panel('Renaming this machine…')).toBeVisible()
        await capture('client-renaming')
        // The hub gave another name than the one asked for: the page shows the one the machine has.
        clientState = 'connected'
        hubName = 'work-2'
        requestedName = null
        pushStatus()
        await expect(page.getByText('joined as')).toContainText('work-2')
        await expect(connection.getByText('The hub named this machine work-2: work is taken or reserved there.', { exact: true })).toBeVisible()
        await expect(panel('Renaming this machine…')).toHaveCount(0)
        // The shared function ids follow the name.
        if (!(await shareRow('shell').locator('details').evaluate((el) => el.open))) await shareRow('shell').locator('summary').click()
        await expect(page.getByText('Ids start with work-2::shell::', { exact: true })).toBeVisible()
        await capture('client-renamed')
        // A refused rename keeps the old name and says why.
        await textbox('Machine name').fill('desk')
        requestedName = 'work'
        clientExtra = { rename_error: 'the hub cannot rename machines; update the hub\'s mesh worker' }
        stored = await savePage({ ...stored, client: { ...stored.client, name: 'desk' } })
        await expect(connection.getByText('Could not rename this machine to desk', { exact: true })).toBeVisible()
        await expect(connection.getByText('the hub cannot rename machines; update the hub\'s mesh worker', { exact: true })).toBeVisible()
        await expect(connection.getByText('It keeps the name work-2.')).toBeVisible()
        await expect(connection.getByText('connected', { exact: true })).toBeVisible()
        await expect(panel('Renaming this machine…')).toHaveCount(0)
        await capture('client-rename-refused')
        // Set back to the name the hub answered for: the worker keeps the old refusal, the page does not show it.
        await textbox('Machine name').fill('work')
        stored = await savePage({ ...stored, client: { ...stored.client, name: 'work' } })
        await expect(connection.getByText(/Could not rename/)).toHaveCount(0)
        await expect(panel('Renaming this machine…')).toHaveCount(0)
        // Cleared, the name asks for nothing: a refusal still in the status until the worker reads the save is not shown.
        await textbox('Machine name').fill('')
        stored = await savePage({ ...stored, client: { ...stored.client, name: '' } })
        await expect(connection.getByText(/Could not rename/)).toHaveCount(0)
        await expect(panel('Renaming this machine…')).toHaveCount(0)
        await textbox('Machine name').fill('desk')
        stored = await savePage({ ...stored, client: { ...stored.client, name: 'desk' } })
        await expect(connection.getByText('Could not rename this machine to desk', { exact: true })).toBeVisible()
        // A ${VAR} name is expanded by the worker only: the page names what it can and invents no pending rename.
        const templated = { ...stored, client: { ...stored.client, name: '${MESH_NAME}' } }
        config = structuredClone(templated)
        pushStatus()
        await expect(connection.getByText('Could not rename this machine', { exact: true })).toBeVisible()
        clientExtra = {}
        requestedName = 'desk'
        hubName = 'desk'
        pushStatus()
        await expect(connection.getByText(/Could not rename/)).toHaveCount(0)
        await expect(connection.getByText('connected', { exact: true })).toBeVisible()
        await expect(panel('Renaming this machine…')).toHaveCount(0)
        await expect(connection.getByText(/The hub named this machine/)).toHaveCount(0)
        // A name the hub had to adjust (set outside this page) is called invalid, not taken.
        config = { ...stored, client: { ...stored.client, name: 'desk-' } }
        requestedName = 'desk-'
        pushStatus()
        await expect(connection.getByText('The hub named this machine desk: desk- is not a valid machine name.', { exact: true })).toBeVisible()
        config = structuredClone(stored)
        requestedName = 'work'
        hubName = 'work-2'
        clientExtra = { rename_error: 'the hub cannot rename machines; update the hub\'s mesh worker' }
        pushStatus()
        await expect(connection.getByText('Could not rename this machine to desk', { exact: true })).toBeVisible()
        // The hub gives the name after all (its worker was updated and this one restarted).
        clientExtra = {}
        hubName = null
        requestedName = null
        pushStatus()
        await expect(page.getByText('joined as')).toContainText('desk')
        await expect(connection.getByText(/Could not rename/)).toHaveCount(0)
        await expect(connection.getByText(/The hub named this machine/)).toHaveCount(0)
        // Another hub URL does not move a joined machine, and the page says what does once the worker reports it unused.
        joinedHub = 'https://hub.example.com'
        await textbox('Hub URL').fill('https://other.example.com')
        stored = await savePage({ ...stored, client: { ...stored.client, hub_url: 'https://other.example.com' } })
        clientExtra = { join_unused: true }
        pushStatus()
        await expect(connection.getByText('Joined to https://hub.example.com. Another Hub URL or join code is used only after Leave hub.', { exact: true })).toBeVisible()
        await expect(page.getByText(/Restart the mesh worker/)).toHaveCount(0)
        await capture('client-joined-elsewhere')
        await textbox('Hub URL').fill('https://hub.example.com')
        stored = await savePage({ ...stored, client: { ...stored.client, hub_url: 'https://hub.example.com' } })
        clientExtra = {}
        pushStatus()
        await expect(connection.getByText(/Joined to/)).toHaveCount(0)
        // The same for a registration the hub has not accepted yet, or a new token behind the same reference: only the worker can tell.
        clientState = 'error'
        clientExtra = { error: 'start mesh: hub did not accept the registration within 1m', retrying: true, join_unused: true }
        pushStatus()
        await expect(connection.getByText('Joined to https://hub.example.com. Another Hub URL or join code is used only after Leave hub.', { exact: true })).toBeVisible()
        clientState = 'connected'
        clientExtra = {}
        joinedHub = null
        pushStatus()
        await expect(connection.getByText(/Joined to/)).toHaveCount(0)
        // A machine the hub removed gets the same action, and a hub that was told leaves no warning.
        clientState = 'removed by hub'
        leaveWarning = ''
        await button('Refresh').click()
        await expect(connection.getByText('removed by hub', { exact: true })).toBeVisible()
        await expect(connection.getByText('the hub removed this machine; leave the hub, then join with a new join code', { exact: true })).toBeVisible()
        await capture('client-removed')
        await button('Leave hub').click()
        await expect(dialog).toContainText('The hub already removed this machine')
        await inDialog('Leave hub').click()
        await expect(connection.getByText('left hub', { exact: true })).toBeVisible()
        await expect(page.getByText('Not sharing')).toHaveCount(0)
        await expect(page.getByText('Left the hub. This engine no longer shares workers.', { exact: true })).toBeAttached()
        clientState = 'connected'
        pushStatus()
        await expect(button('Leave hub')).toBeVisible()
        // Only the mode needs a restart: the worker says so when the stored entry is no longer a client.
        config = { ...stored, mode: 'hub' }
        clientState = 'restart required'
        pushStatus()
        await expect(page.getByText('Restart the mesh worker to switch to hub mode', { exact: true })).toBeVisible()
        config = structuredClone(stored)
        clientState = 'connected'
        pushStatus()
        await expect(page.getByText(/Restart the mesh worker/)).toHaveCount(0)
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
        console.log(`PASS ${viewport.width}px ${theme}: client fields, machine-name validation, share list with the mesh-wide warning, what the mesh shares listed and pushed with both empty states, leave hub confirmed and offered after a removal, leave warning kept on re-read, every join step pushed and announced, saved shares applying and stopping, rename pending, adjusted and refused, hub URL kept while joined, restart only for the mode${timed ? ', nothing polled' : ''}, client-only form`)
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
        // A hub that never applies the saved tunnel: after its deadline the page says so, having read nothing meanwhile.
        // Last in this context, because the fake clock stays installed in it.
        config = { ...initialConfig, tunnel: { enabled: true } }
        applied = initialConfig
        await page.clock.install()
        await page.goto(`${url}/?stall=${randomUUID()}#/worker/mesh/mesh-manager`)
        await expect(tunnelSwitch).toBeChecked()
        const stalled = page.getByText('The hub has not reported a tunnel result. Check the mesh and quick-tunnel workers, then refresh.', { exact: true })
        await expect(stalled).toHaveCount(0)
        const before = statusCalls()
        await page.clock.fastForward(13 * 60_000)
        await expect(stalled).toBeVisible()
        await delay(1000)
        assert.equal(statusCalls(), before, 'the deadline reads nothing')
        applied = null
        console.log(`PASS ${viewport.width}px ${theme}: a hub that goes quiet is named after the deadline, with no polling`)
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
