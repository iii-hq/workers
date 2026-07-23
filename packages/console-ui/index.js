// @iii-dev/console-ui has no bundleable runtime — the real module is served
// by the running console through its import map (/vendor/console-ui.js),
// re-exporting the SPA's own React tree, client, and component library.
// Reaching this file means a worker UI build bundled the package instead of
// leaving it external; fail at import() time with the actual fix.
throw new Error(
  '@iii-dev/console-ui was bundled into this script — it must stay external. ' +
    "Add it to your build's external list (esbuild: --external:@iii-dev/console-ui); " +
    "at runtime the console's import map provides the real module.",
)
