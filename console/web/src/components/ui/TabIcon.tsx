import uiClasses from '@iii-dev/console-ui/ui-classes'
import {
  Activity,
  Braces,
  ChartNoAxesColumnIncreasing,
  CircleAlert,
  Code2,
  Database,
  Eye,
  FileInput,
  FileOutput,
  Files,
  FileText,
  FolderOpen,
  HeartPulse,
  History,
  Info,
  LayoutDashboard,
  Link2,
  ListTree,
  type LucideIcon,
  PanelTop,
  Pencil,
  Play,
  ScrollText,
  Settings2,
  Shield,
  SquareSplitHorizontal,
  Table2,
  Tags,
  Terminal,
  Workflow,
  Zap,
} from 'lucide-react'
import type * as React from 'react'

const TAB_ICONS: Readonly<Record<string, LucideIcon>> = {
  activity: Activity,
  baggage: Braces,
  cell: Braces,
  changes: History,
  config: Settings2,
  configuration: Settings2,
  console: Terminal,
  context: Braces,
  data: Table2,
  diagram: Workflow,
  document: FileText,
  edit: Pencil,
  errors: CircleAlert,
  files: Files,
  fire: Play,
  graph: Workflow,
  health: HeartPulse,
  info: Info,
  input: FileInput,
  invoke: Play,
  json: Braces,
  links: Link2,
  logs: ScrollText,
  'markdown-source': Code2,
  memories: Database,
  network: Link2,
  'otel-logs': ScrollText,
  output: FileOutput,
  overview: LayoutDashboard,
  pages: Files,
  payload: Braces,
  preview: Eye,
  prompt: FileText,
  prompts: FileText,
  row: ListTree,
  rules: FileText,
  settings: Settings2,
  skills: FolderOpen,
  source: Code2,
  split: SquareSplitHorizontal,
  sql: Database,
  stats: ChartNoAxesColumnIncreasing,
  'system-prompt': Shield,
  'system-prompts': Shield,
  system_prompt: Shield,
  tags: Tags,
  terminal: Terminal,
  trigger: Zap,
  triggers: Zap,
  viewport: Eye,
}

function iconKey(value: string) {
  return value.trim().toLowerCase().replaceAll(/\s+/g, '-')
}

export interface DefaultTabIconProps {
  value: string
}

/**
 * Stable semantic fallback for tabs. Known values get a meaningful glyph;
 * new values still receive a neutral panel glyph until a more specific icon
 * is supplied through the tab's `icon` prop.
 */
export function DefaultTabIcon({ value }: DefaultTabIconProps) {
  const Icon = TAB_ICONS[iconKey(value)] ?? PanelTop
  return <Icon aria-hidden className={uiClasses.icon} />
}

export function TabIconSlot({
  icon,
  value,
}: {
  icon?: React.ReactNode | false
  value: string
}) {
  if (icon === false) return null
  return (
    <span className={uiClasses.tabIcon} aria-hidden="true">
      {icon ?? <DefaultTabIcon value={value} />}
    </span>
  )
}
