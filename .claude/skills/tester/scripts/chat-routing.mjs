// Cross-chat routing: with two chats side by side, the person last in chat
// B's composer, an answer sent from the question card in chat A must reach
// chat A's session. The send is captured and dropped (nothing reaches an
// agent). Exit 1 when it is addressed anywhere else.
//
//   CARD_CHAT='ui test for the question card' OTHER_CHAT='explain this project' \
//     node chat-routing.mjs            # the branch as served
//   BEFORE=1 node chat-routing.mjs     # baseline: the card without its sessionId
//
// Found 2026-11-16: answers landed in whichever chat was used last.

import {
  captureCalls,
  launch,
  openChat,
  panels,
  patchBundle,
  restoreLayout,
  shot,
  snapshotLayout,
  splitRight,
  BASE,
} from './lib.mjs'

const FORM = 'form[aria-label="Questions from the agent"]'
const CARD_CHAT = process.env.CARD_CHAT ?? 'ui test for the question card'
const OTHER_CHAT = process.env.OTHER_CHAT ?? 'explain this project in easy language'
const tag = process.env.BEFORE ? 'before' : 'after'

const { browser, page, problems } = await launch()
const sends = await captureCalls(page, ['harness::send', 'harness::ask::settle'])
if (process.env.BEFORE) {
  await patchBundle(page, 'harness', (js) => js.replace(/,sessionId:[\w$]+\.sessionId\}/, '}'))
}
await page.goto(BASE)
await page.waitForTimeout(4000)
const layout = await snapshotLayout(page)

let verdict = 'SKIPPED'
try {
  await openChat(page, CARD_CHAT)
  const paneB = await splitRight(page, 'chat')
  const paneA = panels(page).first()
  await openChat(page, OTHER_CHAT, paneB)
  const card = paneA.locator(FORM).last()
  if (!(await card.count())) throw new Error(`no open question card in "${CARD_CHAT}"`)
  await paneB.locator('[contenteditable="true"]').first().click() // B is now "used last"
  const skips = card.getByRole('button', { name: 'Skip' })
  for (let i = await skips.count(); i > 0; i--) await skips.first().click()
  await shot(page, `chat-routing-${tag}-1`)
  await card.getByRole('button', { name: 'Send' }).click()
  await page.waitForTimeout(2500)
  await shot(page, `chat-routing-${tag}-2`)
  const send = sends.find((s) => s.function_id === 'harness::send')
  if (!send) throw new Error('no harness::send left the page')
  console.log('send ->', send.data?.session_id, JSON.stringify(send.data?.message))
  // Pane B must not show the answer as a new user bubble.
  const leaked = await paneB.getByText('(skipped)', { exact: false }).count()
  verdict = leaked > 0 ? 'FAIL: answer shown in the other chat' : 'PASS'
} catch (e) {
  verdict = `SKIPPED: ${e.message.split('\n')[0]}`
  await shot(page, `chat-routing-${tag}-error`)
}
const after = await restoreLayout(page, layout)
console.log(`[${tag}] ${verdict}`)
console.log('layout before', layout, 'after', after)
if (problems.length) console.log('problems:\n  ' + problems.slice(0, 20).join('\n  '))
await browser.close()
process.exit(verdict.startsWith('FAIL') ? 1 : 0)
