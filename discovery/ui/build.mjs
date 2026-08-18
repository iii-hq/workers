import { readFileSync } from 'node:fs'
import esbuild from 'esbuild'

const watch = process.argv.includes('--watch')
const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  minify: !watch,
  external: [
    'react',
    'react-dom',
    'react-dom/client',
    'react/jsx-runtime',
    '@iii-dev/console-ui',
  ],
  logLevel: 'info',
}

const SCOPE = '[data-iii-ui="discovery"]'
const SCOPE_RE = /^\[data-iii-ui=("discovery"|'discovery'|discovery)\]/

function assertScoped(css) {
  const src = css
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, '""')
  const problems = []
  let depth = 0
  let buf = ''
  const atStack = []

  for (let i = 0; i < src.length; i++) {
    const ch = src[i]
    if (ch === '{') {
      const head = buf.trim()
      buf = ''
      if (head.startsWith('@')) {
        const name = head.split(/[\s(]/)[0]
        atStack.push(name)
        if (name === '@keyframes') problems.push(`@keyframes ${head} is not allowed`)
        if (name === '@font-face') problems.push('@font-face is not allowed in an injected sheet')
        depth++
        continue
      }
      if (!atStack.includes('@keyframes') && head) {
        for (const selector of head.split(',')) {
          const value = selector.trim()
          if (value && !SCOPE_RE.test(value)) problems.push(`selector not scoped: ${value}`)
        }
      }
      depth++
    } else if (ch === '}') {
      depth--
      buf = ''
      if (atStack.length && depth < atStack.length) atStack.pop()
    } else {
      buf += ch
    }
  }

  if (problems.length) {
    console.error(`\n${problems.length} unscoped rule(s) in dist/styles.css:\n`)
    for (const problem of [...new Set(problems)]) console.error(`  ${problem}`)
    console.error(`\nEvery rule must start with ${SCOPE}.\n`)
    process.exit(1)
  }
}

if (watch) {
  const context = await esbuild.context(options)
  await context.watch()
} else {
  await esbuild.build(options)
  assertScoped(readFileSync('dist/styles.css', 'utf8'))
}
