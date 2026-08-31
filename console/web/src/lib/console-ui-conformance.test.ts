/**
 * Drift guards for the `@iii-dev/console-ui` workspace package
 * (packages/console-ui) — the hand-modeled compile-time surface worker UI
 * projects link against, whose runtime is this SPA via the
 * `/vendor/console-ui.js` shim.
 *
 * Two ways they can drift, both pinned here:
 *
 * 1. TYPE-LEVEL (checked by `tsc -b`, this file lives under src/): every
 *    component export the package declares must be satisfied by the real
 *    console component — a declared prop the real component stopped
 *    accepting, or a new required prop the declaration lacks, fails the
 *    console build.
 * 2. NAME-LEVEL (vitest): the curated `components` record, the package's
 *    component-names manifest (which generates the shim's named exports),
 *    and the conformance map below must all agree exactly.
 */

import { readFileSync } from 'node:fs'
import type * as ConsoleUi from '@iii-dev/console-ui'
import componentNames from '@iii-dev/console-ui/component-names'
import tokenNames from '@iii-dev/console-ui/token-names'
import uiClasses, { uiClassNames } from '@iii-dev/console-ui/ui-classes'
import { describe, expect, it } from 'vitest'
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
import { Select } from '@/components/ui/Select'
import { Selector } from '@/components/ui/Selector'
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
import { components } from '@/lib/console-api'
import { Markdown } from '@/lib/markdown'
import { CodeHighlight, JsonHighlight } from '@/lib/syntax'
import { WorkerConfigurationDialog } from '@/pages/Workers/components/WorkerConfigurationDialog'

/**
 * The type-level check: assigning each real component to the package's
 * declared type errors if the declaration drifts. Also the completeness
 * check's data — its keys must equal the manifest.
 */
const conformance: {
  AnnotationLayer: typeof ConsoleUi.AnnotationLayer
  AnnotationList: typeof ConsoleUi.AnnotationList
  AnsiText: typeof ConsoleUi.AnsiText
  Badge: typeof ConsoleUi.Badge
  Button: typeof ConsoleUi.Button
  Card: typeof ConsoleUi.Card
  CardBody: typeof ConsoleUi.CardBody
  CardHighlight: typeof ConsoleUi.CardHighlight
  CardHeader: typeof ConsoleUi.CardHeader
  Chip: typeof ConsoleUi.Chip
  CodeEditor: typeof ConsoleUi.CodeEditor
  CodeHighlight: typeof ConsoleUi.CodeHighlight
  CollapsibleCard: typeof ConsoleUi.CollapsibleCard
  CollapsibleCardContent: typeof ConsoleUi.CollapsibleCardContent
  CollapsibleCardTrigger: typeof ConsoleUi.CollapsibleCardTrigger
  ConfirmDialog: typeof ConsoleUi.ConfirmDialog
  Dialog: typeof ConsoleUi.Dialog
  DialogClose: typeof ConsoleUi.DialogClose
  DialogContent: typeof ConsoleUi.DialogContent
  DialogDescription: typeof ConsoleUi.DialogDescription
  DialogTitle: typeof ConsoleUi.DialogTitle
  DialogTrigger: typeof ConsoleUi.DialogTrigger
  DropdownMenu: typeof ConsoleUi.DropdownMenu
  DropdownMenuContent: typeof ConsoleUi.DropdownMenuContent
  DropdownMenuItem: typeof ConsoleUi.DropdownMenuItem
  DropdownMenuLabel: typeof ConsoleUi.DropdownMenuLabel
  DropdownMenuSeparator: typeof ConsoleUi.DropdownMenuSeparator
  DropdownMenuTrigger: typeof ConsoleUi.DropdownMenuTrigger
  EmptyState: typeof ConsoleUi.EmptyState
  ErrorBoundary: typeof ConsoleUi.ErrorBoundary
  FileDiff: typeof ConsoleUi.FileDiff
  IconButton: typeof ConsoleUi.IconButton
  ImageThumbnailButton: typeof ConsoleUi.ImageThumbnailButton
  ImageViewer: typeof ConsoleUi.ImageViewer
  Input: typeof ConsoleUi.Input
  JsonHighlight: typeof ConsoleUi.JsonHighlight
  List: typeof ConsoleUi.List
  ListGroup: typeof ConsoleUi.ListGroup
  ListGroupLabel: typeof ConsoleUi.ListGroupLabel
  ListItem: typeof ConsoleUi.ListItem
  Markdown: typeof ConsoleUi.Markdown
  MarkdownPreview: typeof ConsoleUi.MarkdownPreview
  ModelPicker: typeof ConsoleUi.ModelPicker
  PageBody: typeof ConsoleUi.PageBody
  PageHeader: typeof ConsoleUi.PageHeader
  PageMain: typeof ConsoleUi.PageMain
  PageShell: typeof ConsoleUi.PageShell
  PageSidebar: typeof ConsoleUi.PageSidebar
  Panel: typeof ConsoleUi.Panel
  PanelBody: typeof ConsoleUi.PanelBody
  PanelHeader: typeof ConsoleUi.PanelHeader
  Select: typeof ConsoleUi.Select
  SegmentedControl: typeof ConsoleUi.SegmentedControl
  Selector: typeof ConsoleUi.Selector
  Skeleton: typeof ConsoleUi.Skeleton
  StatusDot: typeof ConsoleUi.StatusDot
  StatusPanel: typeof ConsoleUi.StatusPanel
  Table: typeof ConsoleUi.Table
  TableBody: typeof ConsoleUi.TableBody
  TableCaption: typeof ConsoleUi.TableCaption
  TableCell: typeof ConsoleUi.TableCell
  TableFooter: typeof ConsoleUi.TableFooter
  TableFrame: typeof ConsoleUi.TableFrame
  TableHead: typeof ConsoleUi.TableHead
  TableHeader: typeof ConsoleUi.TableHeader
  TableRow: typeof ConsoleUi.TableRow
  TableViewport: typeof ConsoleUi.TableViewport
  Tabs: typeof ConsoleUi.Tabs
  TabsContent: typeof ConsoleUi.TabsContent
  TabsList: typeof ConsoleUi.TabsList
  TabsTrigger: typeof ConsoleUi.TabsTrigger
  TerminalCommandLine: typeof ConsoleUi.TerminalCommandLine
  TerminalStream: typeof ConsoleUi.TerminalStream
  Tooltip: typeof ConsoleUi.Tooltip
  TooltipContent: typeof ConsoleUi.TooltipContent
  TooltipTrigger: typeof ConsoleUi.TooltipTrigger
  WorkerConfigurationDialog: typeof ConsoleUi.WorkerConfigurationDialog
} = {
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
  CodeEditor,
  CodeHighlight,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  ConfirmDialog,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  EmptyState,
  ErrorBoundary,
  FileDiff,
  IconButton,
  ImageThumbnailButton,
  ImageViewer,
  Input,
  JsonHighlight,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  Markdown,
  MarkdownPreview,
  ModelPicker,
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
  PageSidebar,
  Panel,
  PanelBody,
  PanelHeader,
  Select,
  SegmentedControl,
  Selector,
  Skeleton,
  StatusDot,
  StatusPanel,
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
  TabsContent,
  TabsList,
  TabsTrigger,
  TerminalCommandLine,
  TerminalStream,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  WorkerConfigurationDialog,
}

const workerBadgeProps: ConsoleUi.BadgeProps = { variant: 'ok' }

describe('@iii-dev/console-ui surface', () => {
  it('the curated components record matches the package manifest', () => {
    expect(Object.keys(components).sort()).toEqual([...componentNames].sort())
  })

  it('every manifest component is type-conformance-checked above', () => {
    expect(Object.keys(conformance).sort()).toEqual([...componentNames].sort())
  })

  it('the record and the named exports are the same objects', () => {
    for (const name of componentNames) {
      expect(components[name], name).toBe(
        conformance[name as keyof typeof conformance],
      )
    }
  })

  it('publishes the positive Badge treatment to worker UIs', () => {
    expect(workerBadgeProps.variant).toBe('ok')
    expect(components.Badge).toBe(Badge)
  })

  it('publishes only tokens declared by the Console theme', () => {
    const themeCss = readFileSync(
      new URL('../index.css', import.meta.url),
      'utf8',
    )
    for (const token of tokenNames) {
      expect(themeCss, token).toContain(`${token}:`)
    }
  })

  it('publishes stable class names backed by shared CSS recipes', () => {
    const recipesCss = readFileSync(
      new URL('../styles/ui-recipes.css', import.meta.url),
      'utf8',
    )
    expect(Object.values(uiClasses)).toEqual(uiClassNames)
    for (const className of uiClassNames) {
      expect(recipesCss, className).toContain(`.${className}`)
    }
  })

  it('publishes the borderless card-highlight inset in both themes', () => {
    const themeCss = readFileSync(
      new URL('../index.css', import.meta.url),
      'utf8',
    )
    const recipesCss = readFileSync(
      new URL('../styles/ui-recipes.css', import.meta.url),
      'utf8',
    )
    const recipe = recipesCss.match(
      /\.iii-ui-card-highlight\s*\{([^}]*)\}/,
    )?.[1]

    expect(themeCss).toContain('--color-card-highlight: #dbdbdb63;')
    expect(themeCss).toContain('--color-card-highlight: #0d0d0e63;')
    expect(recipe).toContain('border: 0;')
    expect(recipe).toContain('background: var(--color-card-highlight);')
  })

  it('publishes the collapsible card transition with shared motion tokens', () => {
    const themeCss = readFileSync(
      new URL('../index.css', import.meta.url),
      'utf8',
    )
    const recipesCss = readFileSync(
      new URL('../styles/ui-recipes.css', import.meta.url),
      'utf8',
    )
    const contentRecipe = recipesCss.match(
      /\.iii-ui-collapsible-card__content\s*\{([^}]*)\}/,
    )?.[1]
    const triggerRecipe = recipesCss.match(
      /\.iii-ui-collapsible-card__trigger\s*\{([^}]*)\}/,
    )?.[1]
    const innerRecipe = recipesCss.match(
      /\.iii-ui-collapsible-card__content-inner\s*\{([^}]*)\}/,
    )?.[1]

    expect(contentRecipe).toContain('grid-template-rows: 0fr;')
    // The caller owns header density (`p-*`). Declaring padding in this
    // unlayered public recipe would override Tailwind utilities in workers.
    expect(triggerRecipe).not.toMatch(/(?:^|\s)padding(?:-[^:]+)?:/)
    expect(contentRecipe).toContain('var(--motion-duration-panel)')
    expect(contentRecipe).toContain('var(--motion-ease-exit)')
    expect(innerRecipe).toContain(
      'transition: transform var(--motion-duration-panel)',
    )
    expect(recipesCss).toContain(
      '.iii-ui-collapsible-card__content[data-state="open"]',
    )
    expect(recipesCss).toContain('grid-template-rows: 1fr;')
    expect(recipesCss).toContain('var(--motion-ease-enter)')
    expect(themeCss).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*?--motion-duration-panel: 0ms;/,
    )
  })

  it('keeps selection recipes neutral instead of accent-colored', () => {
    const recipesCss = readFileSync(
      new URL('../styles/ui-recipes.css', import.meta.url),
      'utf8',
    )
    const selectedBlocks = [
      ...recipesCss.matchAll(
        /[^{}]*(?:data-selected|aria-selected|aria-current|aria-checked)[^{}]*\{([^{}]*)\}/g,
      ),
    ]
    expect(selectedBlocks.length).toBeGreaterThan(0)
    for (const block of selectedBlocks) {
      expect(block[1]).not.toContain('--color-accent')
    }
  })
})
