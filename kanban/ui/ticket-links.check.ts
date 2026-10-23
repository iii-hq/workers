// Self-check for src/links.ts: `node kanban/ui/ticket-links.check.ts` (Node 24 strips types).
import assert from 'node:assert/strict'
import { linkTicketKeys } from './src/links.ts'

const tickets = [
  { key: 'KAN-12', title: 'Fix "quoted" \\ title', status: 'Done' },
  { key: 'KAN-1', title: 'One', status: 'Backlog' },
]
const link = (text: string, key: string, title: string) => `[${text}](#kanban-ticket/${key} "${title}")`
const one = link('KAN-1', 'KAN-1', 'One · Backlog')
const run = (markdown: string, list = tickets) => linkTicketKeys(markdown, list)
const linked = (markdown: string, list = tickets) => (run(markdown, list).match(/\]\(#kanban-ticket\//g) ?? []).length
const unescape = (title: string) => title.replace(/\\([!-\/:-@\[-`{-~])/g, '$1').replace(/&#96;/g, '`')

assert.equal(
  run('see KAN-1 and kan-12.'),
  `see ${one} and ${link('kan-12', 'KAN-12', 'Fix \\"quoted\\" \\\\ title · Done')}.`,
)
assert.equal(linked('KAN-1, (KAN-1) **KAN-1**'), 3, 'start of line, parens, emphasis')
for (const same of [
  'KAN-123 and XKAN-1 and KAN-1x',
  'çKAN-1 and KAN-1é and 日KAN-1',
  'branch fix/KAN-1-login',
  'inline `KAN-1` and ``a `KAN-1` b``',
  '```\nKAN-1\n```',
  '~~~md\nKAN-1\n~~~',
  '[KAN-1](https://example.com/KAN-1) and <https://x.test/KAN-1> and https://x.test/KAN-1',
  '![KAN-1](a.png)',
]) {
  assert.equal(run(same), same, same)
}
assert.equal(linked('_KAN-1_'), 1, 'underscore is not a letter or digit')
assert.equal(
  linkTicketKeys('KAN-9', [{ key: 'KAN-9', title: 'a|b', status: 'Done' }]),
  link('KAN-9', 'KAN-9', 'a\\|b · Done'),
  'a pipe in the title is escaped for GFM tables',
)
assert.equal(run('```\nKAN-1\n```\nKAN-1'), ['```', 'KAN-1', '```', one].join('\n'), 'text after a fence links again')
assert.equal(
  run('```js\nKAN-1\n```` \nKAN-1'),
  ['```js', 'KAN-1', '```` ', one].join('\n'),
  'a longer closing fence still closes',
)
assert.equal(run('```\nKAN-1'), '```\nKAN-1', 'an unclosed fence runs to the end')
assert.equal(run('a ```code``` KAN-1'), 'a ```code``` KAN-1', 'a paragraph with a backtick is left alone')
assert.equal(run('```code``` is inline\nKAN-1'), '```code``` is inline\nKAN-1', 'triple backticks on one line are not a fence, but a backtick')
assert.equal(run('```code``` is inline\n\nKAN-1'), `\`\`\`code\`\`\` is inline\n\n${one}`, 'a blank line ends the paragraph; that line was not a fence')
assert.equal(linkTicketKeys('KAN-1', []), 'KAN-1', 'no linked tickets, no change')

// Titles are untrusted text: whatever they hold stays inside the quotes of the title.
const titled = (title: string) => linkTicketKeys('KAN-1', [{ key: 'KAN-1', title, status: 's' }])
{
  const out = titled('a\n\n![x](https://evil/p.png)')
  assert.ok(!out.includes('\n') && !out.includes('!['), `a blank line and an image in a title stay inert: ${out}`)
  assert.equal(linked(out, [{ key: 'KAN-1', title: 'a', status: 's' }]), 1)
}
for (const title of ['a &amp; b', 'x"y', 'p\\', 'a) KAN-12 b', 'q [r] <s> `t` u | v', '\t tabs\r\n and\u2028lines ']) {
  const out = titled(title)
  const inner = out.slice(out.indexOf(' "') + 2, -2)
  assert.equal(unescape(inner), `${title} · s`.replace(/\s+/g, ' '), `the title round-trips: ${title}`)
  assert.ok(/^(?:[^"\\\n]|\\[!-\/:-@\[-`{-~]|&#96;)*$/.test(inner), `only escapes and plain text between the quotes: ${inner}`)
}

// Keys that are not inert inside a link are not rewritten.
assert.equal(linkTicketKeys('K)-1', [{ key: 'K)-1', title: 't', status: 's' }]), 'K)-1')

// Rewriting twice changes nothing, whatever the titles hold.
{
  const list = [
    { key: 'KAN-1', title: 'a) KAN-12 b', status: 's' },
    { key: 'KAN-12', title: 'x `KAN-1` y [KAN-1](#z) "q" \\', status: 'Done' },
  ]
  for (const text of [
    'KAN-1 KAN-12',
    '`a` KAN-1 `b` KAN-12 `c`',
    '`x KAN-1 and `y` KAN-12',
    '[KAN-1](http://x) KAN-12 **KAN-1** <KAN-1> http://x/KAN-1 www.kan-1.x',
    '```\nKAN-1\n```\nKAN-12 and KAN-1',
  ]) {
    const once = run(text, list)
    assert.equal(run(once, list), once, `idempotent: ${text}`)
  }
}

// Not a mention: an image or an escaped bracket would be written around the key, a reference link or
// definition already is a link, GFM literal autolinks (www., email) already are.
assert.ok(!run('C:\\proj\\KAN-1\\notes').includes('\\['), 'a backslash before the key')
assert.ok(!run('Hi!KAN-1').includes('!['), 'a bang before the key would become an image')
assert.equal(linked('[see KAN-1][r]\n\n[r]: http://x'), 0, 'reference-link text')
assert.equal(linked('x[^KAN-1]\n\n[^KAN-1]: note'), 0, 'footnote labels')
assert.equal(linked('[KAN-1]: https://example.com/a "KAN-1"\nsee KAN-1'), 0, 'a text with a link definition is left alone')
assert.equal(linked('[Note]: see KAN-1'), 0, 'a line with a bracket is left alone')
assert.equal(linked('www.kan-1.example.com a@kan-1.example.com kan-1@example.com'), 0, 'GFM literal autolinks')
assert.equal(linked('[`code` KAN-1](http://x) KAN-1'), 0, 'a line with a bracket or a backtick is left alone')
assert.equal(linked('日**KAN-1**を a**KAN-1** **KAN-1**b'), 0, 'glued emphasis would lose its flanking')
assert.equal(linked('日 **KAN-1** を **(KAN-1)**'), 2, 'emphasis between spaces is fine, so is a marker not touching the key')
assert.equal(linked('(**KAN-1**) **KAN-1**. _~_KAN-1_ ~~_KAN-1 __KAN-1_~'), 0, 'a marker run touching the key must stand against a space')

// Code spans, links, tags: the whole paragraph is left alone, next to them or on a later line.
for (const same of [
  '`x`KAN-1',
  '[a](b)KAN-1',
  '<https://x.y>KAN-1',
  'KAN-1`c`KAN-1',
  '`x ``` KAN-1` y',
  '`` ` KAN-1 ``',
  '``x` KAN-1',
  'a `b\nKAN-1\nc` d',
  '[see\nKAN-1\nhere](http://x)',
  '<a\nhref="KAN-1">\nx</a>',
  'x `y\n- KAN-1',
]) {
  assert.equal(run(same), same, same)
}
assert.equal(run('`x`\n\nKAN-1'), `\`x\`\n\n${one}`, 'the next paragraph is prose again')

// Same boundary as the Rust scanner (`cited_keys`): alphabetic, numeric and combining marks block a key.
assert.deepEqual([linked('ⓐKAN-1'), linked('कीKAN-1'), linked('c\u0327KAN-1'), linked('KAN-1\u0327')], [0, 0, 0, 0])
assert.equal(linked('ſec-1', [{ key: 'SEC-1', title: 't', status: 's' }]), 0, 'long s does not fold to S')
assert.equal(linked('\u212Aan-1 KAN-1'), 1, 'the Kelvin sign does not fold to K')

// Fences inside list items and blockquotes, and the ways a fence line can be something else: the parser, not
// this function, knows where code ends, so linking stops for good.
for (const same of [
  '- a\n  - b\n    ```\n    KAN-1\n    ```',
  '> ```\n> KAN-1\n> ```',
  '10. s\n    ```\n    KAN-1\n    ```',
  '- ```\n  KAN-1\n  ```',
  '1. ~~~\n   KAN-1\n   ~~~',
  '> - ```\n>   KAN-1\n>   ```',
  '- ```\n  KAN-1\n  ```\n\nKAN-1',
  '```\n> ```\nKAN-1\n```',
  '```\ncode\n    ```\nKAN-1\n```',
  '    ```\ntext KAN-1\n\n```\nKAN-1\n```',
  '- a\n  ```\n  code\n- b\n\n```\nKAN-1\n```',
  '- a\n\n      ```\n      x\n\n```\nKAN-1\n```',
  '<div>\n```\nfoo\n\nKAN-1\n```\nKAN-1\n```',
  '  ```\nKAN-1',
]) {
  assert.equal(run(same), same, same)
}
// A fence that starts a line is tracked: what comes after its closing fence is prose, a ~ fence is not closed by a `.
assert.equal(
  run('KAN-1\n```\nKAN-1\n```\nKAN-1\n~~~~\n```\nKAN-1\n~~~\nKAN-1\n~~~~\nKAN-1'),
  [one, '```', 'KAN-1', '```', one, '~~~~', '```', 'KAN-1', '~~~', 'KAN-1', '~~~~', one].join('\n'),
)
assert.equal(run('```\r\nKAN-1\r\n```\r\nKAN-1'), ['```', 'KAN-1', '```', one].join('\r\n'), 'CRLF')
assert.equal(run('a\r```\rKAN-1\r```'), 'a\r```\rKAN-1\r```', 'a lone CR ends a line for the parser')

// HTML is shown as text by the host: a tag, a comment, a block are left alone, so is a fence-looking line in one.
for (const same of [
  'a <!-- KAN-1 --> b',
  '<div>\nKAN-1\n</div>',
  '<!--\nKAN-1\n-->',
  '<!--\n\nKAN-1\n\n-->\n\nKAN-1',
  '- <script>\n\n  KAN-1\n\n  </script>',
  '<abbr title="KAN-1">x</abbr> KAN-1',
  '<pre>\n\nKAN-1\n\n</pre>\n\nKAN-1',
]) {
  assert.equal(run(same), same, same)
}
assert.equal(run('<b>x</b>\n\nKAN-1'), `<b>x</b>\n\n${one}`, 'a blank line ends an HTML block')

// Link definitions: a key can be the label, a reference to it, or the title, wherever it sits.
for (const same of [
  '- [KAN-1]: http://x',
  '> [KAN-1]: http://x',
  '1. [KAN-1]: http://x\nKAN-1',
  '[r]: http://x\n"see KAN-1"\n\n[a][r]',
  '[KAN-1]\n\n[kan-1]: http://x',
  '[x]KAN-1\n\n[kan-1]: http://x',
  'KAN-1\n\n[r]: <http://x>',
]) {
  assert.equal(run(same), same, same)
}

// Left to the parser as well: escapes, indented code (also after a list marker), quotes, addresses.
for (const same of [
  'use \\` then `KAN-1` ok `x`',
  '[a `b](http://x) KAN-1 ` z',
  'a <x`y> KAN-1 ` z',
  'see http://a.b`x KAN-1 ` z',
  '\\[a](b) KAN-1',
  '    KAN-1',
  '\tKAN-1',
  '-     KAN-1',
  '1.     KAN-1',
  '- > KAN-1',
  '> KAN-1',
  '>     KAN-1',
  'KAN-1.foo@x.com @KAN-1 www. KAN-1 awww.KAN-1',
  'a **KAN-1**b a‍**KAN-1**',
]) {
  assert.equal(run(same), same, same)
}
assert.equal(linked('a\\\nKAN-1'), 1, 'only the line with the backslash is left alone')
assert.deepEqual([linked('KAN-1\n\n    code\n\n- KAN-1\n  KAN-1'), linked('# KAN-1\n| KAN-1 | x |\n|-|-|\n| a | KAN-1 |')], [3, 3], 'prose lines of lists, tables, headings')

// No pattern rescans the rest of the line or paragraph. Linear runs take ~25 ms on 200 KB and the old quadratic
// patterns took seconds, so 500 ms catches a regression without failing on a loaded CI machine.
const KB = 200_000
for (const [name, input] of Object.entries({
  'unterminated <': '<'.repeat(KB),
  'unterminated [': '['.repeat(KB),
  'unterminated [a](': '[a]('.repeat(KB / 4),
  'unbalanced destination': `[a](b${'(c)'.repeat(KB / 3)}`,
  'nested parentheses': `[a]${'('.repeat(KB)}`,
  'backticks then one closer': `${'`'.repeat(KB / 2)}x\``,
  'fence with a backtick tail': `\`\`\`${'a'.repeat(KB)}\``,
  'backtick runs 1..630, no closers': Array.from({ length: 630 }, (_, i) => `${'`'.repeat(i + 1)}x`).join(''),
  'backtick runs 630..1': Array.from({ length: 630 }, (_, i) => `${'`'.repeat(630 - i)}x`).join(''),
  'alternating backticks': '`a'.repeat(KB / 2),
  'bare URLs': 'http://'.repeat(KB / 7),
  'reference links': '[a][b'.repeat(KB / 5),
  'one long word': `${'a'.repeat(KB)}KAN-1`,
  'many keys': 'KAN-1 '.repeat(KB / 6),
  'many lines': 'x\n'.repeat(KB / 2),
  'many lines, a key last': `${'x\n'.repeat(KB / 2)}KAN-1`,
  'many lines with keys': 'KAN-1\n'.repeat(KB / 6),
  'many keys, one word': 'KAN-1.'.repeat(KB / 6),
  'many keys between markers': '*KAN-1*'.repeat(KB / 7),
  'marker runs': `${'*'.repeat(KB)}KAN-1${'_'.repeat(KB)}`,
  'quote marks': `${'>'.repeat(KB)}x`,
  'list markers': `${'- '.repeat(KB / 2)}KAN-1`,
  'spaces': `${' '.repeat(KB)}KAN-1`,
  'fence-like lines': '- ```\n'.repeat(KB / 6),
  'fences, one per line': '~~~\n'.repeat(KB / 4),
  'fences with a tail': `\`\`\`\n${'`'.repeat(KB / 2)}x\n\`\`\``,
  'prefixes then a fence': `${'> '.repeat(KB / 4)}\`\`\`${'a'.repeat(KB / 2)}\``,
  'paragraph, a bracket last': `${'KAN-1\n'.repeat(KB / 6)}[`,
  'html lines': '<p>\nKAN-1\n'.repeat(KB / 12),
  'definition-like lines': '[a\n'.repeat(KB / 3),
  'definition label, no end': `[${'a'.repeat(KB)}`,
  'words': 'a '.repeat(KB / 2),
  'tabs and spaces': ' \t'.repeat(KB / 2),
})) {
  // The best of five: a quadratic pattern is a hundred times slower on every run, a busy machine only on some.
  const took = Math.min(
    ...[1, 2, 3, 4, 5].map(() => {
      const started = performance.now()
      run(input)
      return performance.now() - started
    }),
  )
  assert.ok(took < 500, `${name}: ${took.toFixed(1)} ms`)
}

console.log('ticket-links.check: ok')
