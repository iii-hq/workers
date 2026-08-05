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

import type * as ConsoleUi from '@iii-dev/console-ui'
import componentNames from '@iii-dev/console-ui/component-names'
import { describe, expect, it } from 'vitest'
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
import { Input } from '@/components/ui/Input'
import { MarkdownPreview } from '@/components/ui/MarkdownPreview'
import { Select } from '@/components/ui/Select'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
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
  Badge: typeof ConsoleUi.Badge
  Button: typeof ConsoleUi.Button
  CodeEditor: typeof ConsoleUi.CodeEditor
  CodeHighlight: typeof ConsoleUi.CodeHighlight
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
  Input: typeof ConsoleUi.Input
  JsonHighlight: typeof ConsoleUi.JsonHighlight
  Markdown: typeof ConsoleUi.Markdown
  MarkdownPreview: typeof ConsoleUi.MarkdownPreview
  Select: typeof ConsoleUi.Select
  Skeleton: typeof ConsoleUi.Skeleton
  StatusDot: typeof ConsoleUi.StatusDot
  StatusPanel: typeof ConsoleUi.StatusPanel
  Tabs: typeof ConsoleUi.Tabs
  TabsContent: typeof ConsoleUi.TabsContent
  TabsList: typeof ConsoleUi.TabsList
  TabsTrigger: typeof ConsoleUi.TabsTrigger
  Tooltip: typeof ConsoleUi.Tooltip
  TooltipContent: typeof ConsoleUi.TooltipContent
  TooltipTrigger: typeof ConsoleUi.TooltipTrigger
  WorkerConfigurationDialog: typeof ConsoleUi.WorkerConfigurationDialog
} = {
  Badge,
  Button,
  CodeEditor,
  CodeHighlight,
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
  Input,
  JsonHighlight,
  Markdown,
  MarkdownPreview,
  Select,
  Skeleton,
  StatusDot,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  WorkerConfigurationDialog,
}

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
})
