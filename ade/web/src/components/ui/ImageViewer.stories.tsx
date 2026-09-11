import type { Meta, StoryObj } from '@storybook/react-vite'
import { Button } from './Button'
import {
  ImageThumbnailButton,
  ImageViewer,
  type ImageViewerProps,
  useImageViewer,
} from './ImageViewer'

function svgDataUrl(width: number, height: number, label: string): string {
  const cells = 8
  const tiles = Array.from({ length: cells * cells }, (_, i) => {
    const x = (i % cells) * (width / cells)
    const y = Math.floor(i / cells) * (height / cells)
    const fill = (Math.floor(i / cells) + i) % 2 === 0 ? '#b8420f' : '#28a8f7'
    return `<rect x="${x}" y="${y}" width="${width / cells}" height="${height / cells}" fill="${fill}" opacity="0.35"/>`
  }).join('')
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="100%" height="100%" fill="#f2f0ed"/>${tiles}<text x="50%" y="50%" font-family="monospace" font-size="${Math.round(width / 18)}" text-anchor="middle" dominant-baseline="middle" fill="#0a0a0a">${label}</text><circle cx="${width * 0.15}" cy="${height * 0.2}" r="${width * 0.03}" fill="#0a0a0a"/></svg>`
  return `data:image/svg+xml;base64,${btoa(svg)}`
}

const LARGE = svgDataUrl(3200, 2000, '3200 x 2000')
const SMALL = svgDataUrl(320, 200, '320 x 200')

function Demo(props: Omit<ImageViewerProps, 'open' | 'onOpenChange'>) {
  const viewer = useImageViewer()
  return (
    <div className="flex flex-col items-start gap-3">
      <ImageThumbnailButton
        title={props.title ?? 'image'}
        onClick={viewer.show}
      >
        {props.src ? (
          <img
            src={props.src}
            alt=""
            className="h-24 w-auto rounded-xs border border-edge"
          />
        ) : (
          <span className="rounded-xs bg-surface px-3 py-2 font-sans text-[12px] text-ink-faint">
            (no bytes)
          </span>
        )}
      </ImageThumbnailButton>
      <Button variant="pill" size="sm" onClick={viewer.show}>
        Open viewer
      </Button>
      <ImageViewer
        open={viewer.open}
        onOpenChange={viewer.setOpen}
        {...props}
      />
    </div>
  )
}

const meta = {
  title: 'UI/ImageViewer',
  component: Demo,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Demo>

export default meta
type Story = StoryObj<typeof meta>

export const LargeImage: Story = {
  args: {
    src: LARGE,
    alt: 'Checkerboard test image',
    title: 'wireframe-3200.svg',
    description: 'svg · 14kb',
  },
}

export const SmallImage: Story = {
  args: {
    src: SMALL,
    alt: 'Small checkerboard',
    title: 'icon-320.svg',
    description: 'svg · 2kb',
  },
}

export const Unavailable: Story = {
  args: {
    src: null,
    alt: 'Screenshot that was not kept',
    title: 'screenshot.png',
    unavailableReason: 'The image bytes were not kept with this conversation.',
  },
}

export const DecodeFailure: Story = {
  args: {
    src: 'data:image/png;base64,AAAA',
    alt: 'Broken image',
    title: 'broken.png',
  },
}
