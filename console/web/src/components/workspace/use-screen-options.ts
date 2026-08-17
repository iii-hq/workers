import {
  Activity,
  Blocks,
  Boxes,
  Braces,
  BrainCircuit,
  Code,
  Database,
  FileText,
  FolderTree,
  Gauge,
  GitBranch,
  GitPullRequest,
  Globe,
  type LucideIcon,
  MessageSquareText,
  Monitor,
  SquareTerminal,
  Zap,
} from 'lucide-react'
import { buildViewOptions } from '@/lib/nav-options'
import { useExtPages } from '@/lib/ui-slots'
import {
  CHAT_SCREEN,
  screenForExtPage,
  type TabScreen,
} from '@/lib/workspace-tabs'

export interface ScreenOption {
  value: TabScreen
  label: string
  icon: LucideIcon
  description?: string
  keywords?: string[]
}

interface PagePresentation {
  icon: LucideIcon
  description: string
  keywords?: string[]
}

const EXT_PAGE_PRESENTATION: Readonly<Record<string, PagePresentation>> = {
  browser: {
    icon: Globe,
    description: 'Browser sessions and automation.',
  },
  computer: {
    icon: Monitor,
    description: 'Desktop sessions and controls.',
  },
  database: {
    icon: Database,
    description: 'Database connections and queries.',
  },
  directory: {
    icon: FolderTree,
    description: 'Project files and directories.',
    keywords: ['files', 'folders'],
  },
  editor: {
    icon: Code,
    description: 'Files and code editing.',
  },
  'eval-benchmarks': {
    icon: Gauge,
    description: 'Evaluation runs and benchmarks.',
  },
  functions: {
    icon: Braces,
    description: 'Registered runtime functions.',
  },
  github: {
    icon: GitPullRequest,
    description: 'Repositories and pull requests.',
  },
  memory: {
    icon: BrainCircuit,
    description: 'Stored agent memory.',
  },
  'pdf-reader': {
    icon: FileText,
    description: 'PDF documents and pages.',
  },
  shell: {
    icon: SquareTerminal,
    description: 'Interactive shell sessions.',
  },
  state: {
    icon: Database,
    description: 'Shared runtime state.',
  },
  'state-manager': {
    icon: Database,
    description: 'Shared runtime state.',
  },
  triggers: {
    icon: Zap,
    description: 'Event and schedule triggers.',
  },
  worktree: {
    icon: GitBranch,
    description: 'Repository worktrees.',
  },
}

/**
 * Every screen a workspace tab can attach: the chat view, the first-party
 * pages, and the worker-injected pages (whose presence already tracks
 * worker connectedness via trigger GC). Configuration is absent by
 * design — console settings open as an overlay page, not a tab screen.
 */
export function useScreenOptions(): {
  screenOptions: ScreenOption[]
  extPageTitles: ReadonlyMap<string, string>
} {
  const extPages = useExtPages()
  const screenOptions: ScreenOption[] = [
    {
      value: CHAT_SCREEN,
      label: 'chat',
      icon: MessageSquareText,
      description: 'Conversations and tools.',
      keywords: ['assistant', 'agent'],
    },
    ...buildViewOptions().map((option): ScreenOption => {
      if (option.value === 'traces') {
        return {
          ...option,
          icon: Activity,
          description: 'Execution traces and spans.',
          keywords: ['observability', 'logs'],
        }
      }
      return {
        ...option,
        icon: Boxes,
        description: 'Connected runtime workers.',
        keywords: ['runtimes', 'services'],
      }
    }),
    ...extPages.map((page) => {
      const presentation = EXT_PAGE_PRESENTATION[page.id] ?? {
        icon: Blocks,
        description: 'Worker-provided page.',
      }
      return {
        value: screenForExtPage(page.id),
        label: page.title,
        ...presentation,
        keywords: [page.id, page.scope, ...(presentation.keywords ?? [])],
      }
    }),
  ]
  const extPageTitles = new Map(extPages.map((page) => [page.id, page.title]))
  return { screenOptions, extPageTitles }
}
