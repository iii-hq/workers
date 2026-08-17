/**
 * The `@iii-dev/console-ui` surface — what injected scripts reach through
 * the import map's `/vendor/console-ui.js` shim (which re-exports from
 * `window.__III_CONSOLE__.api`; the shim also re-exports every curated
 * component by name, from the manifest in packages/console-ui).
 *
 * `main.tsx` assigns the boot global before anything else runs, then fills
 * `api` in once the shared engine client resolves and starts the loader —
 * injected modules are only ever imported by the loader, so they never
 * observe a null `api`.
 */

import { AnsiText } from '@/components/ui/AnsiText'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { CodeEditor } from '@/components/ui/CodeEditor'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/Dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { EmptyState } from '@/components/ui/EmptyState'
import { ErrorBoundary } from '@/components/ui/ErrorBoundary'
import { FileDiff } from '@/components/ui/FileDiff'
import { Input } from '@/components/ui/Input'
import { MarkdownPreview } from '@/components/ui/MarkdownPreview'
import {
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
  PageSidebar,
} from '@/components/ui/PageChrome'
import { Select } from '@/components/ui/Select'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { TerminalCommandLine } from '@/components/ui/TerminalCommandLine'
import { TerminalStream } from '@/components/ui/TerminalStream'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { useTheme } from '@/hooks/use-theme'
import type { IiiClient } from '@/lib/iii-client'
import { Markdown } from '@/lib/markdown'
import { CodeHighlight, JsonHighlight } from '@/lib/syntax'
import { WorkerConfigurationDialog } from '@/pages/Workers/components/WorkerConfigurationDialog'
import type { ConsoleApi, ExtensionIii } from '@/types/injectable-ui'

/**
 * The curated library. Radix-composed components ship with their parts —
 * a `Dialog` without `DialogContent` would be unusable. Exported for the
 * conformance test, which pins this record to the package's
 * component-names manifest (the shim's export list).
 */
export const components: ConsoleApi['components'] = {
  AnsiText,
  Badge,
  Button,
  Dialog,
  DialogTrigger,
  DialogClose,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  EmptyState,
  ErrorBoundary,
  FileDiff,
  Input,
  PageShell,
  PageHeader,
  PageBody,
  PageSidebar,
  PageMain,
  Select,
  Skeleton,
  StatusDot,
  StatusPanel,
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
  TerminalCommandLine,
  TerminalStream,
  Tooltip,
  TooltipTrigger,
  TooltipContent,
  CodeEditor,
  CodeHighlight,
  JsonHighlight,
  Markdown,
  MarkdownPreview,
  // The one page-level composite in the kit: worker pages offer "configure"
  // in place instead of navigating to the workers tab and stranding the
  // operator there when the editor closes.
  WorkerConfigurationDialog,
}

/** Token names mirroring `index.css`'s `@theme` block (documentation aid). */
const tokens: readonly string[] = [
  '--color-bg',
  '--color-ink',
  '--color-ink-faint',
  '--color-ink-ghost',
  '--color-accent',
  '--color-accent-fg',
  '--color-alert',
  '--color-ok',
  '--color-warn',
  '--color-panel',
  '--color-paper-2',
  '--color-ring',
  '--color-rule',
  '--color-rule-2',
]

/** Reactive theme, without the setter — extensions follow, never drive. */
function useThemeValue(): 'light' | 'dark' {
  const [theme] = useTheme()
  return theme
}

/**
 * Narrow the shared client to the extension surface. TS types erase and
 * `Object.freeze` is shallow, so this is a wrapper object that simply does
 * not carry `dispose` — not the raw `IiiClient`.
 */
export function buildConsoleApi(client: IiiClient): ConsoleApi {
  const iii: ExtensionIii = {
    browserId: client.browserId,
    trigger: (functionId, payload, options) =>
      client.trigger(functionId, payload, options),
    on: (functionId, handler) => client.on(functionId, handler),
    registerTrigger: (input) => client.registerTrigger(input),
    addConnectionStateListener: (handler) =>
      client.addConnectionStateListener(handler),
  }
  return Object.freeze({
    iii: Object.freeze(iii),
    components: Object.freeze({ ...components }),
    useTheme: useThemeValue,
    tokens: Object.freeze([...tokens]),
  })
}
