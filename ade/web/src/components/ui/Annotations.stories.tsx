import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import {
  type Annotation,
  addAnnotation,
  moveAnnotation,
  noteAnnotation,
  removeAnnotation,
} from '@/lib/annotations'
import { AnnotationLayer, AnnotationList } from './Annotations'
import { Button } from './Button'

const WIDTH = 1280
const HEIGHT = 800
const picture = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(
  `<svg xmlns="http://www.w3.org/2000/svg" width="${WIDTH}" height="${HEIGHT}" viewBox="0 0 ${WIDTH} ${HEIGHT}"><rect width="${WIDTH}" height="${HEIGHT}" fill="#f4f1ec"/><rect x="0" y="0" width="${WIDTH}" height="64" fill="#1f1d1a"/><rect x="40" y="120" width="520" height="320" rx="12" fill="#ffffff" stroke="#d9d3ca"/><rect x="600" y="120" width="640" height="140" rx="12" fill="#ffffff" stroke="#d9d3ca"/><rect x="600" y="300" width="640" height="140" rx="12" fill="#ffffff" stroke="#d9d3ca"/><rect x="40" y="480" width="1200" height="260" rx="12" fill="#ffffff" stroke="#d9d3ca"/><text x="40" y="42" font-family="system-ui" font-size="22" fill="#f4f1ec">Pricing</text></svg>`,
)}`

const seeded: Annotation[] = [
  {
    id: 'a',
    x: 0.12,
    y: 0.35,
    note: 'card title wraps on narrow screens',
    label: 'h3.card-title "Starter"',
  },
  { id: 'b', x: 0.78, y: 0.24, note: '', label: 'button.primary "Upgrade"' },
  { id: 'c', x: 0.5, y: 0.78, note: 'this row should be sticky' },
]

function Demo({ initial, active }: { initial: Annotation[]; active: boolean }) {
  const [annotations, setAnnotations] = useState(initial)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [annotating, setAnnotating] = useState(active)
  return (
    <div className="flex h-[520px] gap-3">
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className="flex items-center gap-2">
          <Button
            variant="pill"
            size="sm"
            aria-pressed={annotating}
            onClick={() => setAnnotating((v) => !v)}
          >
            {annotating ? 'Annotating' : 'Annotate'}
          </Button>
          <span className="font-sans text-xs text-ink-faint">
            {annotating
              ? 'click the picture to drop a pin and write its note beside it; drag or arrow a pin to move it; Delete removes'
              : 'pins stay where they are'}
          </span>
        </div>
        <AnnotationLayer
          annotations={annotations}
          image={{ width: WIDTH, height: HEIGHT }}
          active={annotating}
          selectedId={selectedId}
          onAdd={(x, y) =>
            setAnnotations((list) => {
              const next = addAnnotation(list, x, y)
              setSelectedId(next[next.length - 1]?.id ?? null)
              return next
            })
          }
          onSelect={setSelectedId}
          onMove={(id, x, y) =>
            setAnnotations((list) => moveAnnotation(list, id, x, y))
          }
          onRemove={(id) =>
            setAnnotations((list) => removeAnnotation(list, id))
          }
          onNote={(id, note) =>
            setAnnotations((list) => noteAnnotation(list, id, note))
          }
          className="min-h-0 flex-1 rounded-sm border border-edge bg-surface"
        >
          <img
            src={picture}
            alt="a pricing page wireframe"
            draggable={false}
            className="block h-full w-full select-none object-contain"
          />
        </AnnotationLayer>
      </div>
      <div className="w-80 shrink-0 rounded-sm border border-edge bg-panel">
        <AnnotationList
          annotations={annotations}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onNote={(id, note) =>
            setAnnotations((list) => noteAnnotation(list, id, note))
          }
          onRemove={(id) =>
            setAnnotations((list) => removeAnnotation(list, id))
          }
        />
      </div>
    </div>
  )
}

const meta = {
  title: 'UI/Annotations',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

export const Annotating: Story = {
  render: () => <Demo initial={seeded} active />,
}

export const Empty: Story = {
  render: () => <Demo initial={[]} active />,
}

export const Resting: Story = {
  render: () => <Demo initial={seeded} active={false} />,
}
