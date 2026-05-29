import { makeBackend, streamAssistant } from './helpers'

const BODY = `# markdown stress

paragraph with [a regular link](https://example.com) and a bare autolink
<https://example.com/autolink>. inline code: \`useDeferredValue\`. emphasis:
*italic*, **bold**, ***both***, ~~struck~~.

## task list (gfm)

- [x] write the renderer
- [x] handle gfm tables
- [ ] handle footnotes[^note]
- [ ] handle hard breaks  
  this line follows a hard break (two trailing spaces).

## footnotes

here is a sentence with a footnote reference[^1] and another[^2].

[^1]: footnote one — short and sweet.
[^2]: footnote two — multi-line.
    second line of footnote two indented under the marker.
[^note]: a named footnote, just to vary the syntax.

## fenced code variants

\`\`\`ts
type Result<T, E = Error> = { ok: true; value: T } | { ok: false; error: E }

export function ok<T>(value: T): Result<T, never> {
  return { ok: true, value }
}
\`\`\`

\`\`\`python
def fib(n: int) -> int:
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a
\`\`\`

\`\`\`rust
fn main() {
    let xs: Vec<i32> = (1..=10).collect();
    let sum: i32 = xs.iter().sum();
    println!("{}", sum);
}
\`\`\`

## deep nesting

- level one
  - level two
    - level three
      - level four
        - level five — too deep, but renderers must not crash.
- back to level one
  1. ordered nested
  2. inside unordered
     1. and back to ordered three deep.

## quotes

> single-line quote.

> multi-line
> quote that wraps
> across three lines.

>> nested quote with emphasis: *still readable?*

## a busy table

| feature           | status      | notes                           |
|-------------------|-------------|---------------------------------|
| headings          | ok          | h1–h6, all weights              |
| inline code       | ok          | backticks render mono           |
| fenced code       | ok          | language hint drives highlight  |
| tables            | ok          | gfm pipe tables                 |
| task lists        | ok          | checkbox renders                |
| footnotes         | depends     | requires \`remark-gfm\`           |
| autolinks         | ok          | bare urls become anchors        |
| hard breaks       | ok          | trailing two-space → \`<br>\`     |

end of stress.`

export const markdownStress = makeBackend(
  'markdown-stress',
  async function* (_prompt, _mode, _model, opts) {
    yield* streamAssistant(BODY, { signal: opts?.signal, meanDelayMs: 4 })
  },
)
