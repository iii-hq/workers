/**
 * The canonical list of components the console shares with injected worker
 * UI — the single source of truth for:
 *
 * - the `/vendor/console-ui.js` shim's named exports
 *   (console/web/scripts/generate-vendor-shims.mjs), and
 * - the curated `components` record in console/web/src/lib/console-api.ts
 *   (asserted equal by console-ui-conformance.test.ts).
 *
 * Adding a component: add it here, to the record, and to ../index.d.ts —
 * the conformance test and the shim validator catch a miss on either side.
 */
export const componentNames = [
  'Badge',
  'Button',
  'CodeEditor',
  'CodeHighlight',
  'Dialog',
  'DialogClose',
  'DialogContent',
  'DialogDescription',
  'DialogTitle',
  'DialogTrigger',
  'DropdownMenu',
  'DropdownMenuContent',
  'DropdownMenuItem',
  'DropdownMenuLabel',
  'DropdownMenuSeparator',
  'DropdownMenuTrigger',
  'EmptyState',
  'ErrorBoundary',
  'FileDiff',
  'Input',
  'JsonHighlight',
  'Markdown',
  'MarkdownPreview',
  'PageBody',
  'PageHeader',
  'PageMain',
  'PageShell',
  'PageSidebar',
  'Select',
  'Skeleton',
  'StatusDot',
  'StatusPanel',
  'Tabs',
  'TabsContent',
  'TabsList',
  'TabsTrigger',
  'Tooltip',
  'TooltipContent',
  'TooltipTrigger',
  'WorkerConfigurationDialog',
]

export default componentNames
