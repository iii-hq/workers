import esbuild from 'esbuild'

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
}
