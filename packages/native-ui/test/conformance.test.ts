/**
 * The conformance files: up to date with the flows, self-consistent, and the
 * exact strings of design §5.7 (copied here from the design) decode as the
 * design describes them and come out of the harness byte for byte.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'
import { DESIGN_EXAMPLES, FLOWS, generateAll } from '../scripts/gen-conformance.mjs'
import { createDecoder } from '../src/testing/index.mjs'
import { pkg } from './helpers'

const read = (rel: string) => readFileSync(join(pkg, rel), 'utf8')

/** design.md §5.7, verbatim. */
const DESIGN = {
  'counter.ready':
    '[[23,{"sdk":"0.1.0","catalog":1,"features":["ops.json.v0","list.splice.v1","text.handshake.v1","remote-screen.v1"],"pages":[{"name":"root","modal":false,"fullScreen":false}]}]]',
  'counter.init':
    '[[1,{"catalog":1,"features":["ops.json.v0","list.splice.v1","text.handshake.v1","remote-screen.v1"],"host":"android 0.1.0","engine":"quickjs-ng 0.17.0","app":{"key":"worker:demo","ns":"demo"},"env":{"windowClass":"compact","posture":"flat","platform":"android","visible":true,"theme":"dark","locale":"en-US","online":true}}],[6,1,"root",{}]]',
  'counter.mount':
    '[[1,1,40],[2,1,16,"Demo"],[1,2,1],[2,2,16,12],[2,2,8,16],[1,3,10],[2,3,16,"Count 0"],[2,3,17,7],[3,2,0,0,3],[1,4,20],[2,4,16,"Add"],[2,4,17,1],[2,4,2,2],[3,2,0,1,4],[3,1,0,0,2],[7,1,1]]',
  'counter.press': '[[2,4,1,[]]]',
  'counter.update': '[[2,3,16,"Count 1"]]',
  'list.mount':
    '[[1,10,30],[2,10,16,"id"],[1,11,31],[2,11,16,{"$item":"title"}],[2,11,17,{"$item":"url"}],[2,11,21,true],[2,11,2,6],[3,10,1,0,11],[8,10,0,0,[{"id":"s1","title":"Example Domain","url":"example.com"},{"id":"s2","title":"iii","url":"iii.dev"}]]]',
  'list.press': '[[3,10,"s2",11,1,[]]]',
  'list.update': '[[8,10,1,1,[{"id":"s2","title":"iii — More application.","url":"iii.dev"}]]]',
  'text.edit': '[[4,20,2,"ab"]]',
  'text.write': '[[2,20,16,["https://ab",2]]]',
  'requests.send':
    '[[20,7,1,{"fn":"browser::sessions::list","payload":{}}],[20,8,2,{"sub":3,"trigger":"browser::session-started","config":{}}]]',
  'requests.responses': '[[8,8,1,null],[8,7,1,{"sessions":[]}]]',
  'requests.trigger':
    '[[9,3,{"session_id":"s1","url":"about:blank","headless":true,"preview":false,"timestamp":1761151200000}]]',
  'requests.error': '[[8,9,0,{"code":"forbidden","message":"browser may only call browser::… functions"}]]',
  'nav.push': '[[20,12,10,{"page":"console","params":{"session":"s1"}}]]',
  'nav.mount': '[[6,2,"console",{"session":"s1"}]]',
  'nav.back': '[[7,2],[8,12,1,{"result":null}]]',
  'env.change': '[[5,{"windowClass":"wide"}]]',
} as const

/** What design §5.7 says each string means. */
const MEANING: Record<string, unknown[]> = {
  'counter.ready': [
    {
      op: 'ready',
      info: {
        sdk: '0.1.0',
        catalog: 1,
        features: ['ops.json.v0', 'list.splice.v1', 'text.handshake.v1', 'remote-screen.v1'],
        pages: [{ name: 'root', modal: false, fullScreen: false }],
      },
    },
  ],
  'counter.mount': [
    { op: 'create', id: 1, type: 'Page' },
    { op: 'set', id: 1, prop: 'title', value: 'Demo' },
    { op: 'create', id: 2, type: 'Column' },
    { op: 'set', id: 2, prop: 'gap', value: 12 },
    { op: 'set', id: 2, prop: 'padding', value: 16 },
    { op: 'create', id: 3, type: 'Text' },
    { op: 'set', id: 3, prop: 'text', value: 'Count 0' },
    { op: 'set', id: 3, prop: 'role', value: 7, readable: 'title' },
    { op: 'insert', parent: 2, slot: 'children', index: 0, child: 3 },
    { op: 'create', id: 4, type: 'Button' },
    { op: 'set', id: 4, prop: 'text', value: 'Add' },
    { op: 'set', id: 4, prop: 'kind', value: 1, readable: 'primary' },
    { op: 'set', id: 4, prop: 'events', value: 2, readable: ['press'] },
    { op: 'insert', parent: 2, slot: 'children', index: 1, child: 4 },
    { op: 'insert', parent: 1, slot: 'body', index: 0, child: 2 },
    { op: 'root', page: 1, id: 1 },
  ],
  'counter.press': [{ op: 'event', node: 4, event: 'press', args: [] }],
  'counter.update': [{ op: 'set', id: 3, prop: 'text', value: 'Count 1' }],
  'list.mount': [
    { op: 'create', id: 10, type: 'List' },
    { op: 'set', id: 10, prop: 'keyField', value: 'id' },
    { op: 'create', id: 11, type: 'ListRow' },
    { op: 'set', id: 11, prop: 'title', value: { $item: 'title' } },
    { op: 'set', id: 11, prop: 'subtitle', value: { $item: 'url' } },
    { op: 'set', id: 11, prop: 'subtitleMono', value: true },
    { op: 'set', id: 11, prop: 'events', value: 6, readable: ['press', 'longPress'] },
    { op: 'insert', parent: 10, slot: 'template0', index: 0, child: 11 },
    {
      op: 'listSplice',
      list: 10,
      start: 0,
      deleteCount: 0,
      items: [
        { id: 's1', title: 'Example Domain', url: 'example.com' },
        { id: 's2', title: 'iii', url: 'iii.dev' },
      ],
    },
  ],
  'list.press': [{ op: 'itemEvent', list: 10, key: 's2', node: 11, event: 'press', args: [] }],
  'list.update': [
    {
      op: 'listSplice',
      list: 10,
      start: 1,
      deleteCount: 1,
      items: [{ id: 's2', title: 'iii — More application.', url: 'iii.dev' }],
    },
  ],
  'text.edit': [{ op: 'text', node: 20, count: 2, text: 'ab' }],
  'text.write': [{ op: 'set', id: 20, prop: 'value', value: ['https://ab', 2] }],
  'requests.send': [
    { op: 'request', reqId: 7, method: 'call', args: { fn: 'browser::sessions::list', payload: {} } },
    { op: 'request', reqId: 8, method: 'on', args: { sub: 3, trigger: 'browser::session-started', config: {} } },
  ],
  'requests.responses': [
    { op: 'response', reqId: 8, ok: true, value: null },
    { op: 'response', reqId: 7, ok: true, value: { sessions: [] } },
  ],
  'requests.trigger': [
    {
      op: 'trigger',
      sub: 3,
      payload: { session_id: 's1', url: 'about:blank', headless: true, preview: false, timestamp: 1761151200000 },
    },
  ],
  'requests.error': [
    {
      op: 'response',
      reqId: 9,
      ok: false,
      value: { code: 'forbidden', message: 'browser may only call browser::… functions' },
    },
  ],
  'nav.push': [{ op: 'request', reqId: 12, method: 'navPush', args: { page: 'console', params: { session: 's1' } } }],
  'nav.mount': [{ op: 'mount', page: 2, name: 'console', params: { session: 's1' } }],
  'nav.back': [
    { op: 'unmount', page: 2 },
    { op: 'response', reqId: 12, ok: true, value: { result: null } },
  ],
  'env.change': [{ op: 'env', changes: { windowClass: 'wide' } }],
}

describe('design §5.7 examples', () => {
  it('are the ones the generator uses, verbatim', () => {
    expect(Object.fromEntries(DESIGN_EXAMPLES.map((e: { id: string; message: string }) => [e.id, e.message]))).toEqual(
      DESIGN,
    )
  })

  it('are canonical json-v0 (no spaces, integers as integers)', () => {
    for (const message of Object.values(DESIGN)) expect(JSON.stringify(JSON.parse(message))).toBe(message)
  })

  it('decode as the design describes them', () => {
    const decoders = new Map<string, ReturnType<typeof createDecoder>>()
    for (const e of DESIGN_EXAMPLES as {
      id: string
      dir: 'toVm' | 'fromVm'
      message: string
      context?: Record<string, string>
    }[]) {
      const group = e.id.split('.')[0]
      if (!decoders.has(group)) decoders.set(group, createDecoder())
      const decoded = decoders.get(group)!.decode(e.message, e.dir, e.context)
      if (MEANING[e.id]) expect(decoded, e.id).toEqual(MEANING[e.id])
    }
  })

  it('come out of the flows byte for byte', () => {
    for (const e of DESIGN_EXAMPLES as { id: string; dir: 'toVm' | 'fromVm'; message: string; excerpt?: string }[]) {
      const flow = FLOWS.find((f: { designExamples?: string[] }) => f.designExamples?.includes(e.id))
      expect(flow, e.id).toBeTruthy()
      const file = JSON.parse(read(`conformance/${flow!.name}.json`))
      const messages: string[] = file[e.dir]
      if (e.excerpt) {
        const prefix = e.message.slice(0, -1)
        expect(
          messages.some((m) => m.startsWith(`${prefix},`)),
          e.id,
        ).toBe(true)
      } else {
        expect(messages, e.id).toContain(e.message)
      }
    }
  })
})

describe('conformance files', () => {
  let generated: [string, string][]
  beforeAll(async () => {
    generated = await generateAll()
  })

  it('are up to date (run `node scripts/gen-conformance.mjs` after changing the SDK or a flow)', () => {
    for (const [rel, text] of generated) expect(read(rel), rel).toBe(text)
  })

  it('are self-describing and consistent', () => {
    for (const flow of FLOWS) {
      const file = JSON.parse(read(`conformance/${flow.name}.json`))
      expect(file.name).toBe(flow.name)
      expect(file.description.length).toBeGreaterThan(20)
      expect(file.order.filter((d: string) => d === 'toVm').length).toBe(file.toVm.length)
      expect(file.order.filter((d: string) => d === 'fromVm').length).toBe(file.fromVm.length)
      expect(file.order[0]).toBe('fromVm')
      for (const m of [...file.toVm, ...file.fromVm]) expect(JSON.stringify(JSON.parse(m))).toBe(m)
      expect(Object.keys(file.tree).length).toBeGreaterThan(0)
    }
  })

  it('cover the flows the hosts replay', () => {
    expect(FLOWS.map((f: { name: string }) => f.name).sort()).toEqual(
      [
        'action-menu',
        'counter',
        'env',
        'hue',
        'lanes-select',
        'list',
        'list-drop',
        'list-fill',
        'list-splice',
        'markdown',
        'nav-back',
        'nav-result',
        'requests',
        'text',
        'text-handshake',
      ].sort(),
    )
    const handshake = JSON.parse(read('conformance/text-handshake.json'))
    expect(handshake.host.droppedWrites).toEqual([{ node: 3, text: 'filled', count: 1, hostCount: 2, hostText: 'abC' }])
  })
})
