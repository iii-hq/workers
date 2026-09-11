import { PDFDocument } from 'pdf-lib'
import pptxgen from 'pptxgenjs'
import type { SlideImage } from './capture.js'
import type { Deck } from './model.js'
import { SLIDE_HEIGHT, SLIDE_WIDTH } from './render-html.js'

const PPTX_WIDTH = 10
const PPTX_HEIGHT = 5.625

type PptxGenJSClass = typeof pptxgen extends { default: infer D } ? D : typeof pptxgen
const PptxGenJS = ((pptxgen as unknown as { default?: unknown }).default ?? pptxgen) as PptxGenJSClass

function imageBytes(image: SlideImage): Uint8Array {
  return new Uint8Array(Buffer.from(image.data_base64, 'base64'))
}

export async function rasterPdf(deck: Deck, images: SlideImage[]): Promise<Uint8Array> {
  const doc = await PDFDocument.create()
  doc.setTitle(deck.title)
  if (deck.author) doc.setAuthor(deck.author)
  doc.setProducer('iii slides')
  for (const image of images) {
    const page = doc.addPage([SLIDE_WIDTH, SLIDE_HEIGHT])
    const embedded =
      image.content_type === 'image/png' ? await doc.embedPng(imageBytes(image)) : await doc.embedJpg(imageBytes(image))
    page.drawImage(embedded, { x: 0, y: 0, width: SLIDE_WIDTH, height: SLIDE_HEIGHT })
  }
  return doc.save()
}

export async function rasterPptx(deck: Deck, images: SlideImage[]): Promise<Buffer> {
  const pptx = new PptxGenJS()
  pptx.layout = 'LAYOUT_16x9'
  pptx.title = deck.title
  if (deck.author) pptx.author = deck.author
  for (const image of images) {
    const source = deck.slides[image.index]
    const slide = pptx.addSlide()
    slide.addImage({
      data: `${image.content_type};base64,${image.data_base64}`,
      x: 0,
      y: 0,
      w: PPTX_WIDTH,
      h: PPTX_HEIGHT,
    })
    if (source?.notes) slide.addNotes(source.notes)
  }
  const output = await pptx.write({ outputType: 'nodebuffer' })
  return Buffer.isBuffer(output) ? output : Buffer.from(output as ArrayBuffer)
}
