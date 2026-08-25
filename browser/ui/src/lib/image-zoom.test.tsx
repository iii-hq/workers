import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ZoomableImage } from './image-zoom'

describe('ZoomableImage', () => {
  it('renders the thumbnail as a pressable image, closed by default', () => {
    const html = renderToStaticMarkup(
      <ZoomableImage
        src="data:image/png;base64,AAAA"
        alt="capture of example.com"
        className="br-ui-shot-img"
      />,
    )
    expect(html).toContain('role="button"')
    expect(html).toContain('tabindex="0"')
    expect(html).toContain('capture of example.com')
    expect(html).not.toContain('zoom-out')
  })
})
