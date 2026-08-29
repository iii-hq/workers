import * as esbuild from 'esbuild';

const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', '@iii-dev/console-ui'],
  logLevel: 'info',
};

if (process.argv.includes('--watch')) {
  const ctx = await esbuild.context(options);
  await ctx.watch();
  console.log('watching aspire-dashboard ui');
} else {
  await esbuild.build(options);
}
