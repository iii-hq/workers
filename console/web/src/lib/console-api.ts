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

import tokenNames from '@iii-dev/console-ui/token-names'
import sharedUiClasses from '@iii-dev/console-ui/ui-classes'
import { DirectoryPicker } from '@/components/chat/DirectoryPicker'
import { ModelPicker } from '@/components/chat/ModelPicker'
import { AnnotationLayer, AnnotationList } from '@/components/ui/Annotations'
import { AnsiText } from '@/components/ui/AnsiText'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { Chip } from '@/components/ui/Chip'
import { CodeEditor } from '@/components/ui/CodeEditor'
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@/components/ui/CollapsibleCard'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
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
import { IconButton } from '@/components/ui/IconButton'
import { ImageThumbnailButton, ImageViewer } from '@/components/ui/ImageViewer'
import { Input } from '@/components/ui/Input'
import { List, ListGroup, ListGroupLabel, ListItem } from '@/components/ui/List'
import { MarkdownPreview } from '@/components/ui/MarkdownPreview'
import { SegmentedControl } from '@/components/ui/ModeToggle'
import {
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
  PageSidebar,
} from '@/components/ui/PageChrome'
import { RawValueInput } from '@/components/ui/RawValueInput'
import { Select } from '@/components/ui/Select'
import { Selector } from '@/components/ui/Selector'
import {
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
} from '@/components/ui/Settings'
import { SettingsDeck } from '@/components/ui/SettingsDeck'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import {
  Card,
  CardBody,
  CardHeader,
  CardHighlight,
  Panel,
  PanelBody,
  PanelHeader,
} from '@/components/ui/Surface'
import { Switch } from '@/components/ui/Switch'
import {
  Table,
  TableBody,
  TableCaption,
  TableCell,
  TableFooter,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@/components/ui/Table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { TerminalCommandLine } from '@/components/ui/TerminalCommandLine'
import { TerminalStream } from '@/components/ui/TerminalStream'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { Wordmark } from '@/components/ui/Wordmark'
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
  AnnotationLayer,
  AnnotationList,
  AnsiText,
  Badge,
  Button,
  Card,
  CardBody,
  CardHighlight,
  CardHeader,
  Chip,
  ConfirmDialog,
  Dialog,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
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
  IconButton,
  ImageThumbnailButton,
  ImageViewer,
  Input,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  PageShell,
  PageHeader,
  PageBody,
  PageSidebar,
  PageMain,
  Panel,
  PanelBody,
  PanelHeader,
  RawValueInput,
  Select,
  SegmentedControl,
  Selector,
  SettingsDeck,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Skeleton,
  StatusDot,
  StatusPanel,
  Switch,
  Table,
  TableBody,
  TableCaption,
  TableCell,
  TableFooter,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
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
  ModelPicker,
  // Compatibility bridge for older worker bundles. New pages set
  // `configurationId`; the Console supplies the PageHeader action and opens
  // the worker inside global Settings.
  WorkerConfigurationDialog,
  DirectoryPicker,
  Wordmark,
}

/** Public token/recipe inventories are canonicalized in the workspace package. */
export const tokens: readonly string[] = tokenNames
export const uiClasses = sharedUiClasses

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
    uiClasses,
  })
}
