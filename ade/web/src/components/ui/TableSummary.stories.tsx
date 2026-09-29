import type { Meta, StoryObj } from '@storybook/react-vite'
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
} from './Table'

const workers = [
  { name: 'browser', sessions: 2, memoryMb: 412 },
  { name: 'coder', sessions: 3, memoryMb: 268 },
  { name: 'ide', sessions: 1, memoryMb: 96 },
]
const totals = workers.reduce(
  (sum, worker) => ({
    sessions: sum.sessions + worker.sessions,
    memoryMb: sum.memoryMb + worker.memoryMb,
  }),
  { sessions: 0, memoryMb: 0 },
)

/* Table.stories covers the body; this file is the caption and footer. */
const meta = {
  title: 'UI/TableSummary',
  component: Table,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Table>

export default meta
type Story = StoryObj<typeof meta>

function Workers({ caption, footer }: { caption?: boolean; footer?: boolean }) {
  return (
    <div className="max-w-xl">
      <TableViewport>
        <TableFrame>
          <Table aria-label="Live workers">
            {caption ? (
              <TableCaption>Live workers, refreshed 4 s ago</TableCaption>
            ) : null}
            <TableHeader>
              <TableRow>
                <TableHead>Worker</TableHead>
                <TableHead className="text-right">Sessions</TableHead>
                <TableHead className="text-right">Memory</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {workers.map((worker) => (
                <TableRow key={worker.name}>
                  <TableCell className="font-code">{worker.name}</TableCell>
                  <TableCell className="text-right">
                    {worker.sessions}
                  </TableCell>
                  <TableCell className="text-right">
                    {worker.memoryMb} MB
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
            {footer ? (
              <TableFooter>
                <TableRow>
                  <TableCell>Total</TableCell>
                  <TableCell className="text-right">
                    {totals.sessions}
                  </TableCell>
                  <TableCell className="text-right">
                    {totals.memoryMb} MB
                  </TableCell>
                </TableRow>
              </TableFooter>
            ) : null}
          </Table>
        </TableFrame>
      </TableViewport>
    </div>
  )
}

/** `TableCaption` names the table for everyone; `TableFooter` carries totals. */
export const CaptionAndFooter: Story = {
  render: () => <Workers caption footer />,
}

export const FooterOnly: Story = { render: () => <Workers footer /> }
