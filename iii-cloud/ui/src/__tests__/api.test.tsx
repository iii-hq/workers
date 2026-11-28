// iii cloud reads and writes as the person (U1; frozen with api.ts).
import type { Host } from '@iii-dev/console-ui'
import { errorCode } from '@iii-dev/console-ui/format'
import { act } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { type Cloud, cloudPath, cloudWrite, useCloud } from '../api'
import { apiPath, base, ENV, following, TEAM } from '../fixtures/base'
import { plainMessage } from '../model'
import type { Status } from '../types'
import { fakeHost } from './host'
import { flush, root, usePage } from './util'

vi.mock('@iii-dev/console-ui', async () => await import('./console-ui-mock'))
vi.mock('@iii-dev/console-ui/hooks', async () => await import('./hooks-mock'))

usePage()

type Seen = Cloud<unknown>[]
function Probe(p: {
  host: Host
  status: Status
  env: string | null
  path: string | null
  pollMs?: number
  seen: Seen
}) {
  p.seen.push(useCloud(p.host, p.status, p.env, p.path, { pollMs: p.pollMs }))
  return null
}
const show = async (p: Parameters<typeof Probe>[0]) => {
  await act(async () => root.render(<Probe {...p} />))
  await flush()
}
const apiCalls = (h: ReturnType<typeof fakeHost>) => h.calls.filter((c) => c.id === 'iii-cloud::api')

describe('useCloud', () => {
  it('GETs the env route, the query string as the query object; a null env reads the team route', async () => {
    const h = fakeHost(base, { 'iii-cloud::api': () => ({ status: 200, body: { lines: ['ok'] } }) })
    const seen: Seen = []
    await show({ host: h.host, status: base, env: ENV.id, path: '/logs?deployment_id=d1&source=release', seen })
    expect(apiCalls(h)[0].payload).toEqual({
      method: 'GET',
      path: apiPath('/logs'),
      query: { deployment_id: 'd1', source: 'release' },
    })
    expect(seen.at(-1)).toMatchObject({ data: { lines: ['ok'] }, error: null, loading: false })
    await show({ host: h.host, status: base, env: null, path: '/members', seen })
    expect(apiCalls(h)[1].payload).toEqual({ method: 'GET', path: `/teams/${TEAM.id}/members` })
    await show({ host: h.host, status: base, env: ENV.id, path: null, seen })
    expect(apiCalls(h)).toHaveLength(2)
    expect(seen.at(-1)).toMatchObject({ data: null, loading: false })
  })

  it('an answer to an older request never overwrites a newer one', async () => {
    let release: (v: unknown) => void = () => undefined
    const slow = new Promise((r) => {
      release = r
    })
    const h = fakeHost(base, {
      'iii-cloud::api': (p: Record<string, unknown>) =>
        p.path === apiPath('/health') ? slow : { status: 200, body: 'machines' },
    })
    const seen: Seen = []
    await show({ host: h.host, status: base, env: ENV.id, path: '/health', seen })
    expect(seen.at(-1)).toMatchObject({ data: null, loading: true })
    await show({ host: h.host, status: base, env: ENV.id, path: '/machines', seen })
    expect(seen.at(-1)).toMatchObject({ data: 'machines', loading: false })
    await act(async () => release({ status: 200, body: 'health' }))
    await flush()
    expect(seen.at(-1)).toMatchObject({ data: 'machines', error: null })
  })

  it('a non-2xx answer is a plain message; a 403 keeps the role the cloud names; a refused call too', async () => {
    const answers: Record<string, { status: number; body: unknown }> = {
      [apiPath('/gone')]: { status: 404, body: { code: 'NOT_FOUND', message: 'No such environment.' } },
      [apiPath('/admin')]: { status: 403, body: { code: 'FORBIDDEN', message: 'This needs the Admin role.' } },
      [apiPath('/bare')]: { status: 500, body: null },
    }
    const refused = Object.assign(new Error('NOT_CONNECTED: no session'), { code: 'NOT_CONNECTED' })
    const h = fakeHost(base, {
      'iii-cloud::api': (p: Record<string, unknown>) => {
        if (p.path === apiPath('/refused')) throw refused
        return answers[String(p.path)]
      },
    })
    const seen: Seen = []
    await show({ host: h.host, status: base, env: ENV.id, path: '/gone', seen })
    expect(seen.at(-1)?.error).toEqual({
      code: 'NOT_FOUND',
      status: 404,
      message: plainMessage({ code: 'NOT_FOUND', message: '' }),
    })
    expect(seen.at(-1)?.error?.message).not.toContain('NOT_FOUND')
    await show({ host: h.host, status: base, env: ENV.id, path: '/admin', seen })
    expect(seen.at(-1)?.error).toMatchObject({ code: 'FORBIDDEN', status: 403, message: 'This needs the Admin role.' })
    await show({ host: h.host, status: base, env: ENV.id, path: '/bare', seen })
    expect(seen.at(-1)?.error).toMatchObject({
      code: 'CLOUD_ERROR',
      message: plainMessage({ code: 'CLOUD_ERROR', message: '' }),
    })
    await show({ host: h.host, status: base, env: ENV.id, path: '/refused', seen })
    expect(seen.at(-1)?.error).toMatchObject({
      code: 'NOT_CONNECTED',
      message: 'You are not signed in to iii cloud. Sign in again.',
    })
  })

  it('reads again when a deploy of its environment changes and when the status is refreshed, not for other envs', async () => {
    const h = fakeHost(base, { 'iii-cloud::api': () => ({ status: 200, body: {} }) })
    const seen: Seen = []
    const at = (status: Status) => show({ host: h.host, status, env: ENV.id, path: '/deployments', seen })
    await at(base)
    expect(apiCalls(h)).toHaveLength(1)
    await at({ ...base })
    expect(apiCalls(h)).toHaveLength(1)
    await at({ ...base, deploys: [following] })
    expect(apiCalls(h)).toHaveLength(2)
    await at({ ...base, deploys: [{ ...following, outcome: 'ready', status: 'ready' }] })
    expect(apiCalls(h)).toHaveLength(3)
    const other = { ...following, id: 'dep-other', environment_id: 'e-other' }
    await at({ ...base, deploys: [{ ...following, outcome: 'ready', status: 'ready' }, other] })
    expect(apiCalls(h)).toHaveLength(3)
    await at({ ...base, deploys: [{ ...following, outcome: 'ready', status: 'ready' }, other], refreshed_at: 'later' })
    expect(apiCalls(h)).toHaveLength(4)
  })

  it('pollMs reads again on a timer, and refetch on demand', async () => {
    const h = fakeHost(base, { 'iii-cloud::api': () => ({ status: 200, body: {} }) })
    const seen: Seen = []
    await show({ host: h.host, status: base, env: ENV.id, path: '/logs', pollMs: 10, seen })
    // One act per tick: inside a single act React would batch every tick into one read.
    for (let i = 0; i < 3; i++) await act(async () => void (await new Promise((r) => setTimeout(r, 15))))
    const polled = apiCalls(h).length
    expect(polled).toBeGreaterThanOrEqual(3)
    await show({ host: h.host, status: base, env: ENV.id, path: '/logs', seen })
    await act(async () => seen.at(-1)?.refetch())
    await flush()
    expect(apiCalls(h).length).toBeGreaterThan(polled)
  })
})

describe('cloudWrite', () => {
  it('sends the method, the env path and the body, with a fresh idempotency_key every call', async () => {
    const h = fakeHost(base, { 'iii-cloud::api::write': () => ({ status: 202, body: { id: 'dom-1' } }) })
    await expect(cloudWrite(h.host, base, ENV.id, 'POST', '/domains', { hostname: 'a.example.com' })).resolves.toEqual({
      id: 'dom-1',
    })
    await cloudWrite(h.host, base, ENV.id, 'PATCH', '', { settings: { release: null } })
    const writes = h.calls.filter((c) => c.id === 'iii-cloud::api::write').map((c) => c.payload ?? {})
    expect(writes[0]).toMatchObject({ method: 'POST', path: apiPath('/domains'), body: { hostname: 'a.example.com' } })
    expect(writes[1]).toMatchObject({ method: 'PATCH', path: apiPath(''), body: { settings: { release: null } } })
    const keys = writes.map((w) => w.idempotency_key)
    for (const k of keys) expect(k).toMatch(/^[0-9a-f-]{36}$/)
    expect(new Set(keys).size).toBe(2)
  })

  it('a refusal rejects with the code, the status and a plain message', async () => {
    const h = fakeHost(base, {
      'iii-cloud::api::write': () => ({ status: 409, body: { code: 'CONFLICT', message: 'That domain is taken.' } }),
    })
    const err = await cloudWrite(h.host, base, ENV.id, 'POST', '/domains', { hostname: 'x' }).catch((e) => e)
    expect(errorCode(err)).toBe('CONFLICT')
    expect(err).toMatchObject({ status: 409, message: 'That domain is taken.' })
    const none = { ...base, project: { ...base.project!, team: null } }
    await expect(cloudWrite(h.host, none, ENV.id, 'POST', '/domains')).rejects.toMatchObject({ code: 'NO_TEAM' })
  })
})

describe('useCloud on a whole /teams/… path', () => {
  it('reads again only when a deploy of the environment in its path changes', async () => {
    const h = fakeHost(base, { 'iii-cloud::api': () => ({ status: 200, body: { deployments: [] } }) })
    const seen: Seen = []
    const path = `/teams/t2/environments/${ENV.id}/deployments`
    const at = (status: Status) => show({ host: h.host, status, env: null, path, seen })
    await at(base)
    expect(apiCalls(h)).toHaveLength(1)
    // A deploy of another environment (another row on Start): nothing to read.
    await at({ ...base, deploys: [{ ...following, environment_id: 'e-other' }] })
    expect(apiCalls(h)).toHaveLength(1)
    await at({ ...base, deploys: [following] })
    expect(apiCalls(h)).toHaveLength(2)
    // A whole path with no environment (a project's environments) never follows deploys.
    const list = '/teams/t2/projects/p/environments'
    await show({ host: h.host, status: base, env: null, path: list, seen })
    const n = apiCalls(h).length
    await show({
      host: h.host,
      status: { ...base, deploys: [{ ...following, id: 'd-x' }] },
      env: null,
      path: list,
      seen,
    })
    expect(apiCalls(h)).toHaveLength(n)
  })
})

describe('cloudPath', () => {
  it('a whole /teams/… path with no env passes through: Start reads other teams', () => {
    expect(cloudPath(base, null, '/teams/t2/projects/p-billing/environments')).toBe(
      '/teams/t2/projects/p-billing/environments',
    )
    expect(cloudPath({ ...base, project: null }, null, '/teams/t2/environments/e/deployments')).toBe(
      '/teams/t2/environments/e/deployments',
    )
    expect(cloudPath(base, ENV.id, '/deployments')).toBe(apiPath('/deployments'))
  })

  it("without the project's team, an API key's team; with neither, nothing to read", async () => {
    const key = { secret: 'IIICLOUD_API_KEY', team_id: 'tk', environment_id: ENV.id }
    const keyed: Status = {
      ...base,
      connection: { ...base.connection, method: 'api_key', api_key: key },
      project: { ...base.project!, team: null },
    }
    expect(cloudPath(keyed, ENV.id, '/deployments')).toBe(`/teams/tk/environments/${ENV.id}/deployments`)
    expect(cloudPath({ ...keyed, connection: base.connection }, ENV.id, '/deployments')).toBeNull()
    // The env cards read through the key's team instead of waiting on a team forever.
    const h = fakeHost(keyed, { 'iii-cloud::api': () => ({ status: 200, body: { deployments: [] } }) })
    const seen: Seen = []
    await show({ host: h.host, status: keyed, env: ENV.id, path: '/deployments', seen })
    expect(apiCalls(h).map((c) => c.payload?.path)).toEqual([`/teams/tk/environments/${ENV.id}/deployments`])
    expect(seen.at(-1)?.data).toEqual({ deployments: [] })
  })
})
