import { strict as assert } from 'node:assert'
import { quickTunnelPlan } from '../quick-tunnel.ts'

// quick-tunnel's entry as its manifest installs it.
const installed = {
  provider: 'cloudflared', targets: { webhooks: 'http://127.0.0.1:3112' }, cloudflared: 'cloudflared', state_path: 'data/quick-tunnel/leases.json',
  startup_timeout_ms: 30000, max_retries: 3, retry_initial_ms: 1000, retry_max_ms: 30000, max_leases: 1024, max_lease_seconds: 2592000,
}
const setUp = { ...installed, provider: 'opentunnel', targets: { webhooks: 'http://127.0.0.1:3112', mesh: 'http://127.0.0.1:49180' }, proxy_protocol: ['mesh'], startup_timeout_ms: 120000 }

// Installed with cloudflared and github's webhooks target: every change is listed, webhooks is named, nothing else moves.
let plan = quickTunnelPlan(installed, 'mesh', '0.0.0.0:49180')
assert.equal(plan.ok, false)
assert.equal(plan.blocked, '')
assert.deepEqual(plan.changes, [
  'use OpenTunnel instead of cloudflared', 'add the target mesh: http://127.0.0.1:49180',
  'pass each visitor\'s address to the hub (proxy_protocol: mesh)', 'allow 2 minutes for the tunnel to start',
])
assert.deepEqual(plan.others, ['webhooks'])
assert.deepEqual(plan.next, setUp)
assert.deepEqual(installed.targets, { webhooks: 'http://127.0.0.1:3112' }, 'the read value is not mutated')

// Already set up: nothing to change, and the value is the one read.
plan = quickTunnelPlan(setUp, 'mesh', '0.0.0.0:49180')
assert.deepEqual([plan.ok, plan.changes, plan.others, plan.blocked], [true, [], [], ''])
assert.deepEqual(plan.next, setUp)

// A fresh entry with no keys: quick-tunnel's built-in webhooks target survives the new list; no lease cap is added.
plan = quickTunnelPlan({}, 'mesh', '0.0.0.0:49180')
assert.deepEqual(plan.next, { provider: 'opentunnel', targets: { webhooks: 'http://127.0.0.1:3112', mesh: 'http://127.0.0.1:49180' }, proxy_protocol: ['mesh'], startup_timeout_ms: 120000 })
assert.deepEqual(plan.others, ['webhooks'])

// Templates the fix does not need stay as written; other proxy_protocol ids stay; a short lease cap is raised.
const templated = {
  provider: 'opentunnel', cloudflared: '${CLOUDFLARED_BIN:cloudflared}', state_path: '${QT_STATE}',
  targets: { webhooks: '${WEBHOOK_ORIGIN}', api: 'http://127.0.0.1:8080' }, proxy_protocol: ['api'], startup_timeout_ms: 300000, max_lease_seconds: 300,
}
plan = quickTunnelPlan(templated, 'mesh', '0.0.0.0:49180')
assert.deepEqual(plan.changes, ['add the target mesh: http://127.0.0.1:49180', 'pass each visitor\'s address to the hub (proxy_protocol: mesh)', 'let the hub keep the tunnel for 10 minutes at a time'])
assert.deepEqual(plan.others, [], 'no provider switch: no other public URL changes')
assert.deepEqual(plan.next, { ...templated, targets: { ...templated.targets, mesh: 'http://127.0.0.1:49180' }, proxy_protocol: ['api', 'mesh'], max_lease_seconds: 600 })

// A custom tunnel id and port; a target already there under the id is pointed at the hub.
plan = quickTunnelPlan({ ...setUp, targets: { hub: 'http://127.0.0.1:9000' }, proxy_protocol: [] }, 'hub', '127.0.0.1:7443')
assert.deepEqual(plan.changes, ['point the target hub at http://127.0.0.1:7443', 'pass each visitor\'s address to the hub (proxy_protocol: hub)'])
assert.deepEqual(plan.next.targets, { hub: 'http://127.0.0.1:7443' })
assert.deepEqual(plan.next.proxy_protocol, ['hub'])
for (const listen of ['[::]:49180', ':49180', 'localhost:49180']) assert.equal(quickTunnelPlan(setUp, 'mesh', listen).ok, true, `${listen} answers on 127.0.0.1`)
// A trailing slash is the same origin to quick-tunnel.
assert.equal(quickTunnelPlan({ ...setUp, targets: { ...setUp.targets, mesh: 'http://127.0.0.1:49180/' } }, 'mesh', '0.0.0.0:49180').ok, true)
// Numbers written as strings are compared as numbers, and ${VAR} templates are left as written: never replaced, never lowered.
for (const [start, lease] of [['180000', '2592000'], ['${QT_START:180000}', '${LEASE}']]) {
  plan = quickTunnelPlan({ ...setUp, startup_timeout_ms: start, max_lease_seconds: lease }, 'mesh', '0.0.0.0:49180')
  assert.deepEqual([plan.ok, plan.changes], [true, []], `${start} and ${lease} are kept`)
  assert.deepEqual([plan.next.startup_timeout_ms, plan.next.max_lease_seconds], [start, lease])
}
plan = quickTunnelPlan({ ...setUp, startup_timeout_ms: '30000', max_lease_seconds: '300' }, 'mesh', '0.0.0.0:49180')
assert.deepEqual([plan.next.startup_timeout_ms, plan.next.max_lease_seconds], [120000, 600], 'a short numeric string is raised')
// Another target on the hub's port may stay when it already takes each visitor's address.
assert.equal(quickTunnelPlan({ ...setUp, targets: { ...setUp.targets, api: 'http://127.0.0.1:49180' }, proxy_protocol: ['mesh', 'api'] }, 'mesh', '0.0.0.0:49180').ok, true)

// What the page must not fix: a listener the tunnel cannot reach, a port it cannot read, an id OpenTunnel refuses, raw values,
// and another target OpenTunnel cannot carry.
const blocked = (config, id, listen, text) => {
  const p = quickTunnelPlan(config, id, listen)
  assert.equal(p.ok, false)
  assert.equal(p.next, null, `${listen} ${id}: nothing to write`)
  assert.match(p.blocked, text)
}
blocked(installed, 'mesh', '203.0.113.20:49180', /The tunnel forwards to 127\.0\.0\.1, but the hub listens on 203\.0\.113\.20:49180/)
blocked(installed, 'mesh', '[::1]:49180', /but the hub listens on \[::1\]:49180/)
blocked(installed, 'mesh', '${MESH_LISTEN:0.0.0.0:49180}', /cannot tell its port/)
blocked(installed, 'Mesh_Hub', '0.0.0.0:49180', /cannot name an OpenTunnel target/)
blocked('${QUICK_TUNNEL_CONFIG}', 'mesh', '0.0.0.0:49180', /raw value/)
blocked({ ...installed, targets: '${TARGETS}' }, 'mesh', '0.0.0.0:49180', /raw value/)
blocked({ ...installed, targets: { webhooks: 'https://127.0.0.1:3112' } }, 'mesh', '0.0.0.0:49180', /OpenTunnel cannot carry quick-tunnel's webhooks target/)
blocked({ ...installed, proxy_protocol: '${PROXY_PROTOCOL}' }, 'mesh', '0.0.0.0:49180', /raw value/)
for (const listen of ['0.0.0.0:0', '0.0.0.0:65536']) blocked(installed, 'mesh', listen, /which the tunnel cannot forward to/)
// What quick-tunnel's validate() refuses: a port shared in and out of proxy_protocol, and a 33rd target.
blocked({ ...installed, targets: { webhooks: 'http://127.0.0.1:49180' } }, 'mesh', '0.0.0.0:49180', /webhooks target also uses port 49180/)
const many = Object.fromEntries(Array.from({ length: 32 }, (_, i) => [`t${i}`, `http://127.0.0.1:${4000 + i}`]))
blocked({ ...setUp, targets: many }, 'mesh', '0.0.0.0:49180', /already carries 32 targets/)
const { t0, ...fewer } = many
assert.equal(quickTunnelPlan({ ...setUp, targets: { ...fewer, mesh: 'http://127.0.0.1:1' } }, 'mesh', '0.0.0.0:49180').blocked, '', 'pointing the 32nd target at the hub adds none')
console.log('PASS quickTunnelPlan: fresh, cloudflared with webhooks, already set up, templates and numeric strings kept, custom id and port, proxy_protocol merged, blocked cases')
