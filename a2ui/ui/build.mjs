import esbuild from 'esbuild'
import { statSync } from 'node:fs'

const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: [
    'react',
    'react-dom',
    'react-dom/client',
    'react/jsx-runtime',
    '@iii-dev/console-ui',
  ],
  logLevel: 'info',
}

if (process.argv.includes('--watch')) {
  const context = await esbuild.context(options)
  await context.watch()
} else {
  await esbuild.build(options)
  const cap = 8 * 1024 * 1024
  for (const file of ['dist/page.js', 'dist/styles.css']) {
    const bytes = statSync(file).size
    console.log(`${file}: ${bytes} bytes`)
    if (bytes >= cap) {
      console.error(`${file} exceeds the Console 8 MiB asset cap`)
      process.exitCode = 1
    }
  }
}
