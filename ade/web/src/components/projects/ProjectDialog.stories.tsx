import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import type { ChatProject } from '@/lib/backend/projects'
import type { ProjectDialog } from './ProjectDialog'

const NOW = Date.now()

const ACME: ChatProject = {
  id: 'prj_acme_web',
  name: 'acme-web',
  color: 'blue',
  folders: [
    '/home/me/work/acme/web',
    '/home/me/work/acme/api',
    '/home/me/work/acme/infra',
  ],
  created_at_ms: NOW,
  updated_at_ms: NOW,
}
const BILLING: ChatProject = {
  ...ACME,
  id: 'prj_billing',
  name: 'billing-api',
  color: 'purple',
  folders: ['/home/me/work/billing'],
}

const CONVERSATIONS = [
  { id: 'a', workingDir: '/home/me/work/acme/web' },
  { id: 'b', workingDir: '/home/me/work/acme/web' },
  { id: 'c', workingDir: '/home/me/work/acme/web/src' },
  { id: 'd', workingDir: '/home/me/work/acme/api' },
]

/* The Project-Settings board, without its account and auto-compact
   sections (phases 4 and 3). Saving is a no-op here. */
const meta = {
  title: 'Chat/ProjectDialog',
  parameters: { layout: 'centered' },
  args: {
    open: true,
    onOpenChange: fn(),
    projects: [BILLING],
    conversations: CONVERSATIONS,
    save: async (input: { name: string }) => ({
      project: { ...ACME, name: input.name },
      moved: 4,
    }),
  },
} satisfies Meta<typeof ProjectDialog>

export default meta
type Story = StoryObj<typeof meta>

/** "New project" with three folders and "Bring the 4 conversations…". */
export const NewProject: Story = {
  args: {
    initialFolders: ACME.folders,
  },
}

/** "Edit project": its values, "Save". */
export const EditProject: Story = {
  args: {
    project: ACME,
    projects: [ACME, BILLING],
  },
}
