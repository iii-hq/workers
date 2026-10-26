#!/usr/bin/env node
/**
 * conformance/apps/*.tsx → conformance/*.json, by running each flow in the
 * test harness.
 *
 *   node scripts/gen-conformance.mjs          write the files
 *   node scripts/gen-conformance.mjs --check  exit 1 when they are stale
 *
 * Each file is one flow, self-describing, for the hosts to replay in their
 * own unit tests: `toVm` and `fromVm` are the exact json-v0 messages of each
 * direction, `order` interleaves them (`order[i]` says which list the i-th
 * message of the transcript comes from), `tree` is the host's tree of every
 * card at the end, `host` what else the host did (writes it dropped, toasts,
 * menus). `designExamples` names the §5.7 strings the flow reproduces;
 * design-examples.json holds those strings with their decoded form.
 */

import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildNativeUi } from '../src/build.mjs'
import { createDecoder, createTestHost } from '../src/testing/index.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

const STARTED = {
  session_id: 's1',
  url: 'about:blank',
  headless: true,
  preview: false,
  timestamp: 1761151200000,
}

/** The §5.7 strings (json-v0), exactly as the design writes them. */
export const DESIGN_EXAMPLES = [
  {
    id: 'counter.ready',
    dir: 'fromVm',
    message:
      '[[23,{"sdk":"0.1.0","catalog":1,"features":["ops.json.v0","list.splice.v1","text.handshake.v1","remote-screen.v1"],"pages":[{"name":"root","modal":false,"fullScreen":false}]}]]',
  },
  {
    id: 'counter.init',
    dir: 'toVm',
    message:
      '[[1,{"catalog":1,"features":["ops.json.v0","list.splice.v1","text.handshake.v1","remote-screen.v1"],"host":"android 0.1.0","engine":"quickjs-ng 0.17.0","app":{"key":"worker:demo","ns":"demo"},"env":{"windowClass":"compact","posture":"flat","platform":"android","visible":true,"theme":"dark","locale":"en-US","online":true}}],[6,1,"root",{}]]',
  },
  {
    id: 'counter.mount',
    dir: 'fromVm',
    message:
      '[[1,1,40],[2,1,16,"Demo"],[1,2,1],[2,2,16,12],[2,2,8,16],[1,3,10],[2,3,16,"Count 0"],[2,3,17,7],[3,2,0,0,3],[1,4,20],[2,4,16,"Add"],[2,4,17,1],[2,4,2,2],[3,2,0,1,4],[3,1,0,0,2],[7,1,1]]',
  },
  { id: 'counter.press', dir: 'toVm', message: '[[2,4,1,[]]]' },
  { id: 'counter.update', dir: 'fromVm', message: '[[2,3,16,"Count 1"]]' },
  {
    id: 'list.mount',
    dir: 'fromVm',
    message:
      '[[1,10,30],[2,10,16,"id"],[1,11,31],[2,11,16,{"$item":"title"}],[2,11,17,{"$item":"url"}],[2,11,21,true],[2,11,2,6],[3,10,1,0,11],[8,10,0,0,[{"id":"s1","title":"Example Domain","url":"example.com"},{"id":"s2","title":"iii","url":"iii.dev"}]]]',
    excerpt: 'The ops of the List; in a whole message the List is then attached (here [7,1,10], the root of page 1).',
  },
  { id: 'list.press', dir: 'toVm', message: '[[3,10,"s2",11,1,[]]]' },
  {
    id: 'list.update',
    dir: 'fromVm',
    message: '[[8,10,1,1,[{"id":"s2","title":"iii — More application.","url":"iii.dev"}]]]',
  },
  { id: 'text.edit', dir: 'toVm', message: '[[4,20,2,"ab"]]', context: { 20: 'TextField' } },
  { id: 'text.write', dir: 'fromVm', message: '[[2,20,16,["https://ab",2]]]', context: { 20: 'TextField' } },
  {
    id: 'requests.send',
    dir: 'fromVm',
    message:
      '[[20,7,1,{"fn":"browser::sessions::list","payload":{}}],[20,8,2,{"sub":3,"trigger":"browser::session-started","config":{}}]]',
  },
  { id: 'requests.responses', dir: 'toVm', message: '[[8,8,1,null],[8,7,1,{"sessions":[]}]]' },
  {
    id: 'requests.trigger',
    dir: 'toVm',
    message:
      '[[9,3,{"session_id":"s1","url":"about:blank","headless":true,"preview":false,"timestamp":1761151200000}]]',
  },
  {
    id: 'requests.error',
    dir: 'toVm',
    message: '[[8,9,0,{"code":"forbidden","message":"browser may only call browser::… functions"}]]',
  },
  { id: 'nav.push', dir: 'fromVm', message: '[[20,12,10,{"page":"console","params":{"session":"s1"}}]]' },
  { id: 'nav.mount', dir: 'toVm', message: '[[6,2,"console",{"session":"s1"}]]' },
  { id: 'nav.back', dir: 'toVm', message: '[[7,2],[8,12,1,{"result":null}]]' },
  { id: 'env.change', dir: 'toVm', message: '[[5,{"windowClass":"wide"}]]' },
]

/** The host of the catalogue 2 flows: the app as it is now. */
const HOST = { host: 'android 0.1.0', engine: 'quickjs-ng 0.17.0' }

/** The host of the §5.7 strings: an app of catalogue 1, as the design writes its `init`. */
const DESIGN_HOST = { ...HOST, catalog: 1 }

/** The flows; each runs on a fresh host with the app built from conformance/apps/<app>.tsx. */
export const FLOWS = [
  {
    name: 'counter',
    app: 'counter',
    description:
      'Design §5.7, a counter: Page "Demo" > Column (gap 12, padding 16) > Text "Count 0" (role title), Button "Add" (primary, press). init and mount go in one message; one press and the text update.',
    host: { ns: 'demo', ...DESIGN_HOST },
    designExamples: ['counter.ready', 'counter.init', 'counter.mount', 'counter.press', 'counter.update'],
    async run(h) {
      await h.mount('root')
      await h.press({ type: 'Button', text: 'Add' })
    },
  },
  {
    name: 'list',
    app: 'list',
    description:
      'Design §5.7, a list: List 10 (keyField "id") as the root of page 1, template 0 = ListRow 11 with title and url bound, subtitle mono, press and longPress (mask 6); the items arrive in one listSplice. A tap on the second row changes its title (one splice of one item), a long press removes the first.',
    host: { ns: 'demo', ...DESIGN_HOST, ids: { node: 10 } },
    designExamples: ['list.mount', 'list.press', 'list.update'],
    async run(h) {
      await h.mount('root')
      await h.itemPress({ type: 'List' }, 's2')
      await h.itemLongPress({ type: 'List' }, 's1')
    },
  },
  {
    name: 'list-splice',
    app: 'list-splice',
    description:
      'A List with two templates chosen by kindField "kind" (rows and section headers), header, empty and footer slots, a trailing IconButton in the row template. A trigger replaces the items four times (first fill, an append plus a changed title, a removal in the middle, a reorder) and createList().set sends only the splices that changed; then a tap on a row and on the close button inside a row.',
    host: { ns: 'demo', ...DESIGN_HOST },
    async run(h) {
      await h.mount('root')
      const header = { key: 'h-tabs', kind: 1, title: 'TABS' }
      const a = { key: 'a', kind: 0, title: 'Example Domain', url: 'example.com/' }
      const b = { key: 'b', kind: 0, title: 'iii', url: 'iii.dev' }
      const c = { key: 'c', kind: 0, title: 'GitHub', url: 'github.com/login' }
      await h.emitTrigger('demo::tabs-changed', { rows: [header, a, b] })
      await h.emitTrigger('demo::tabs-changed', { rows: [header, a, { ...b, title: 'iii — More application.' }, c] })
      await h.emitTrigger('demo::tabs-changed', { rows: [header, a, c] })
      await h.emitTrigger('demo::tabs-changed', { rows: [header, c, a] })
      await h.itemPress('tabs', 'a')
      await h.itemPress('tabs', 'c', 'close')
    },
  },
  {
    name: 'text',
    app: 'text',
    description:
      'Design §5.7, text: TextField 20 is the root of page 1; the person types "a" then "ab" in one turn (one text entry with count 2) and the script normalizes it to "https://ab", written with that count.',
    host: { ns: 'demo', ...DESIGN_HOST, ids: { node: 20 } },
    designExamples: ['text.edit', 'text.write'],
    async run(h) {
      await h.mount('root')
      await h.type({ type: 'TextField' }, ['a', 'ab'])
    },
  },
  {
    name: 'text-handshake',
    app: 'text-handshake',
    description:
      "The edit-count handshake: the script lower-cases each edit (written with the count it saw); then a trigger makes the script write \"filled\" with count 1 while the person had typed again (count 2): the host drops that write (see host.droppedWrites), the person's text reaches the script as the next text entry, and the script's answer with count 2 applies. Submit sends the host's text.",
    host: { ns: 'demo', ...DESIGN_HOST },
    async run(h) {
      await h.mount('root')
      await h.type('name', 'Ab')
      h.typeWhileBusy('name', 'abC')
      await h.emitTrigger('demo::fill', { text: 'filled' })
      await h.submit('name')
    },
  },
  {
    name: 'requests',
    app: 'requests',
    description:
      "Design §5.7, a call, a subscription, an error (namespace browser; ids seeded to request 7 and subscription 3): a press sends a call and an on in one message; the host answers the on at once and the call after; the trigger arrives; a second press calls engine::functions::list, outside the namespace, and the host's policy answers forbidden. iii.call refuses that call before the wire, so the app sends it raw (the harness's test-only rawRequests).",
    host: { ns: 'browser', ...DESIGN_HOST, ids: { req: 7, sub: 3 }, rawRequests: true },
    calls: {
      'browser::sessions::list': () => ({ sessions: [] }),
    },
    designExamples: ['requests.send', 'requests.responses', 'requests.trigger', 'requests.error'],
    async run(h) {
      await h.mount('root')
      await h.press({ type: 'Button', text: 'Load' })
      await h.emitTrigger('browser::session-started', STARTED)
      await h.press({ type: 'Button', text: 'Engine' })
    },
  },
  {
    name: 'nav-back',
    app: 'nav',
    description:
      'Design §5.7, navigation: the root (page 1) pushes "console" with {session: "s1"} (request 12); the host mounts it as page 2; Back unmounts it and answers the push with {result: null}; the script destroys the page\'s nodes.',
    host: { ns: 'browser', ...DESIGN_HOST, ids: { req: 12 } },
    designExamples: ['nav.push', 'nav.mount', 'nav.back'],
    async run(h) {
      await h.mount('root')
      await h.press({ type: 'Button', text: 'Console' })
      await h.back()
    },
  },
  {
    name: 'nav-result',
    app: 'nav',
    description:
      'Navigation with a result: the root pushes "console" (a fullScreen page); the console pops itself with {picked: "error"} (navPop); the host unmounts it and answers the push with that result, which the root shows.',
    host: { ns: 'browser', ...DESIGN_HOST },
    async run(h) {
      await h.mount('root')
      await h.press({ type: 'Button', text: 'Console' })
      await h.press({ type: 'Button', text: 'Done' })
    },
  },
  {
    name: 'env',
    app: 'env',
    description: 'Design §5.7, env: the window passes 720 dp; only the key that changed travels, and the page follows.',
    host: { ns: 'demo', ...DESIGN_HOST },
    designExamples: ['env.change'],
    async run(h) {
      await h.mount('root')
      await h.env({ windowClass: 'wide' })
    },
  },
  {
    name: 'action-menu',
    app: 'action-menu',
    description:
      'An action menu anchored to the IconButton in the Page actions slot (anchor = its node id): title, subtitle, three actions (one with a hint, one destructive); the host answers index 2, and the page shows the choice.',
    host: { ns: 'demo', ...DESIGN_HOST },
    ui: { actionMenu: () => 2 },
    async run(h) {
      await h.mount('root')
      await h.press({ type: 'IconButton', label: 'More' })
    },
  },
  {
    name: 'markdown',
    app: 'markdown',
    description:
      'Catalogue 2, Markdown (type 19): a dense, selectable Markdown with a heading, a list, a code block, a web link and an app link (kan:KAN-12), subscribed to link. The person taps the app link, then the web link: each arrives as link [href] (event 14), the host opens nothing, and the script shows the last href.',
    host: { ns: 'demo', ...HOST },
    async run(h) {
      await h.mount('root')
      await h.link('notes', 'kan:KAN-12')
      await h.link('notes', 'https://example.com')
    },
  },
  {
    name: 'lanes-select',
    app: 'board',
    description:
      'Catalogue 2, Lanes (type 33) in a ListDetail with listFill: titles Todo/Doing/Done with counts, one List per lane in slot 0 (dragGroup "cards", drop subscribed), cards with a Badge whose hue is bound to the item (the script\'s names travel as indexes). The person settles on lane 2, then on lane 1 (select [index], event 5): the host shows it at once, and the script sets selected to the same lane.',
    host: { ns: 'demo', ...HOST },
    async run(h) {
      await h.mount('root')
      await h.select('lanes', 2)
      await h.select('lanes', 1)
    },
  },
  {
    name: 'list-drop',
    app: 'board',
    description:
      'Catalogue 2, drag and drop (List.dragGroup, drop [key, index, source], event 15): KAN-1 is dropped at index 2 of its own lane (a reorder: the index counts the items without it), then KAN-2 from lane 0 to index 1 of lane 1, then KAN-4 from lane 1 to the empty lane 2. Each time the script splices both lists in the handler (one message with the listSplice ops and the new counts).',
    host: { ns: 'demo', ...HOST },
    async run(h) {
      await h.mount('root')
      await h.drop('lane-0', 'KAN-1', 2)
      await h.drop('lane-1', 'KAN-2', 1, 'lane-0')
      await h.drop('lane-2', 'KAN-4', 0, 'lane-1')
    },
  },
  {
    name: 'list-fill',
    app: 'board',
    description:
      'Catalogue 2, ListDetail.listFill (prop 19): the board fills the width while no card is open. A tap on KAN-4 in lane 1 sets Lanes.selected to 1 and showDetail to true in the same message, with the detail page; dismiss closes it and the board fills the width again.',
    host: { ns: 'demo', ...HOST },
    async run(h) {
      await h.mount('root')
      await h.itemPress('lane-1', 'KAN-4')
      await h.emit('board', 'dismiss')
    },
  },
  {
    name: 'hue',
    app: 'hue',
    description:
      'Catalogue 2, named hues (enum hue: none, blue, purple, teal, green, amber, rose): an Icon (prop 19), a Badge (19) and a StatusDot (19) in each hue, by index on the wire (none, the default, does not travel); a Chip (21) in blue turns green on a press.',
    host: { ns: 'demo', ...HOST },
    async run(h) {
      await h.mount('root')
      await h.press('chip')
    },
  },
]

/** JSON as biome formats it: objects expanded, arrays on one line when they fit in 120 columns. */
export function formatJson(value, indent = '') {
  const inner = `${indent}  `
  if (Array.isArray(value)) {
    if (value.length === 0) return '[]'
    const flat = value.every((v) => v === null || typeof v !== 'object' || (Array.isArray(v) && isFlat(v)))
    if (flat) {
      const line = `[${value.map((v) => formatJson(v, inner)).join(', ')}]`
      if (indent.length + line.length <= 116) return line
    }
    return `[\n${value.map((v) => inner + formatJson(v, inner)).join(',\n')}\n${indent}]`
  }
  if (value && typeof value === 'object') {
    const keys = Object.keys(value)
    if (!keys.length) return '{}'
    return `{\n${keys.map((k) => `${inner}${JSON.stringify(k)}: ${formatJson(value[k], inner)}`).join(',\n')}\n${indent}}`
  }
  return JSON.stringify(value)
}

const isFlat = (a) => a.every((v) => v === null || typeof v !== 'object' || (Array.isArray(v) && isFlat(v)))

async function buildApps(names) {
  const dir = mkdtempSync(join(tmpdir(), 'native-ui-conformance-'))
  const bundles = new Map()
  try {
    for (const name of names) {
      const outfile = join(dir, `${name}.js`)
      const ns = FLOWS.find((f) => f.app === name).host.ns
      await buildNativeUi({ entry: join(root, 'conformance/apps', `${name}.tsx`), outfile, ns, smoke: false })
      bundles.set(name, readFileSync(outfile, 'utf8'))
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
  return bundles
}

/** Runs one flow; resolves with its conformance file content (an object). */
export async function runFlow(flow, bundle) {
  const host = await createTestHost({ bundle, ...flow.host, calls: flow.calls, ui: flow.ui })
  try {
    await flow.run(host)
    const tree = {}
    for (const card of host.deck) tree[card.inst] = host.tree(card.inst)
    const extra = {}
    if (host.droppedWrites.length) extra.droppedWrites = host.droppedWrites
    if (host.toasts.length) extra.toasts = host.toasts
    if (host.menus.length) extra.menus = host.menus
    if (host.errors.length) extra.errors = host.errors
    const out = {
      name: flow.name,
      description: flow.description,
      catalog: host.ready.catalog,
      encoding: 'json-v0',
      app: `conformance/apps/${flow.app}.tsx`,
      ns: flow.host.ns,
      toVm: host.transcript.filter((t) => t.dir === 'toVm').map((t) => t.message),
      fromVm: host.transcript.filter((t) => t.dir === 'fromVm').map((t) => t.message),
      order: host.transcript.map((t) => t.dir),
      tree,
    }
    if (Object.keys(extra).length) out.host = extra
    if (flow.designExamples) out.designExamples = flow.designExamples
    return out
  } finally {
    host.dispose()
  }
}

/** design-examples.json: the §5.7 strings, each with its decoded form and the flow that reproduces it. */
export function designExamplesFile() {
  const decoders = new Map()
  const examples = DESIGN_EXAMPLES.map((e) => {
    const group = e.id.split('.')[0]
    if (!decoders.has(group)) decoders.set(group, createDecoder())
    const decoder = decoders.get(group)
    const flow = FLOWS.find((f) => f.designExamples?.includes(e.id))
    const out = { id: e.id, dir: e.dir, message: e.message }
    if (e.excerpt) out.excerpt = e.excerpt
    if (e.context) out.context = e.context
    out.decoded = decoder.decode(e.message, e.dir, e.context)
    out.flow = `${flow.name}.json`
    return out
  })
  return {
    name: 'design-examples',
    description:
      'The exact json-v0 strings of design §5.7 (the same as the host decoding tests), each with its decoded form (named ops; enum and event names under "readable") and the conformance flow that reproduces it byte for byte (an excerpt: as the prefix of a message).',
    catalog: 1,
    examples,
  }
}

/**
 * Every conformance file: [relative path, content as text].
 * @returns {Promise<[string, string][]>}
 */
export async function generateAll() {
  const bundles = await buildApps([...new Set(FLOWS.map((f) => f.app))])
  /** @type {[string, string][]} */
  const files = []
  for (const flow of FLOWS) {
    files.push([`conformance/${flow.name}.json`, `${formatJson(await runFlow(flow, bundles.get(flow.app)))}\n`])
  }
  files.push(['conformance/design-examples.json', `${formatJson(designExamplesFile())}\n`])
  return files
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const files = await generateAll()
  if (process.argv.includes('--check')) {
    const stale = files.filter(([rel, text]) => {
      try {
        return readFileSync(join(root, rel), 'utf8') !== text
      } catch {
        return true
      }
    })
    if (stale.length) {
      console.error(
        `stale conformance files (run node scripts/gen-conformance.mjs): ${stale.map(([r]) => r).join(', ')}`,
      )
      process.exit(1)
    }
  } else {
    for (const [rel, text] of files) writeFileSync(join(root, rel), text)
    console.log(`wrote ${files.length} conformance files`)
  }
}
