import type { Meta, StoryObj } from '@storybook/react-vite'
import { Tabs, TabsContent, TabsList, TabsTrigger } from './Tabs'

const meta = {
  title: 'UI/Tabs',
  component: Tabs,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Tabs>

export default meta
type Story = StoryObj<typeof meta>

export const Line: Story = {
  args: { defaultValue: 'overview' },
  render: (args) => (
    <Tabs {...args} className="max-w-xl">
      <TabsList variant="line">
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="activity">Activity</TabsTrigger>
        <TabsTrigger value="settings">Settings</TabsTrigger>
      </TabsList>
      <TabsContent value="overview" className="pt-4 font-sans text-sm">
        Content tabs use a neutral underline, sans labels and 16px icons.
      </TabsContent>
      <TabsContent value="activity" className="pt-4 font-sans text-sm">
        Activity
      </TabsContent>
      <TabsContent value="settings" className="pt-4 font-sans text-sm">
        Settings
      </TabsContent>
    </Tabs>
  ),
}
