import type { Meta, StoryObj } from '@storybook/react-vite'
import { Markdown } from '@/lib/markdown'
import { Chip } from './Chip'
import {
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from './Table'

const fields = [
  {
    field: 'description',
    required: true,
    type: 'string',
    notes: 'Description shown in the configuration list.',
  },
  {
    field: 'id',
    required: true,
    type: 'string',
    notes: 'Stable configuration identifier.',
  },
  {
    field: 'initial_value',
    required: false,
    type: 'unknown',
    notes: 'Optional initial value validated against the schema.',
  },
]

const meta = {
  title: 'UI/Table',
  component: Table,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Table>

export default meta
type Story = StoryObj<typeof meta>

export const Schema: Story = {
  render: () => (
    <div className="max-w-4xl">
      <TableViewport>
        <TableFrame>
          <Table aria-label="Configuration fields">
            <TableHeader>
              <TableRow>
                <TableHead className="w-[30%]">Field</TableHead>
                <TableHead className="w-[18%]">Type</TableHead>
                <TableHead>Notes</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {fields.map((field) => (
                <TableRow key={field.field}>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <code className="font-code font-medium text-ink">
                        {field.field}
                      </code>
                      {field.required ? (
                        <Chip className="border border-edge bg-transparent font-semibold">
                          Required
                        </Chip>
                      ) : null}
                    </div>
                  </TableCell>
                  <TableCell>
                    <Chip className="border border-edge font-code">
                      {field.type}
                    </Chip>
                  </TableCell>
                  <TableCell className="text-ink-faint">
                    {field.notes}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableFrame>
      </TableViewport>
    </div>
  ),
}

export const NarrowScrollable: Story = {
  render: () => (
    <div className="max-w-80">
      <TableViewport>
        <TableFrame>
          <Table className="min-w-[36rem]" aria-label="Configuration fields">
            <TableHeader>
              <TableRow>
                <TableHead>Field</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Notes</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {fields.map((field) => (
                <TableRow key={field.field}>
                  <TableCell className="font-code">{field.field}</TableCell>
                  <TableCell className="font-code">{field.type}</TableCell>
                  <TableCell>{field.notes}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </TableFrame>
      </TableViewport>
    </div>
  ),
}

export const ChatMarkdown: Story = {
  render: () => (
    <div className="max-w-xl">
      <Markdown>{`A compact table inside a chat response:

| Field | Type | Notes |
| --- | --- | --- |
| \`path\` | \`string\` | Absolute or workspace-relative path. |
| \`recursive\` | \`boolean\` | Removes nested entries when enabled. |
| \`timeout_ms\` | \`number\` | Optional execution timeout. |`}</Markdown>
    </div>
  ),
}
