import type { Deck } from '../../src/model.js'

export function sampleDeck(): Deck {
  return {
    id: 'deck-test0001',
    title: 'Quarterly review',
    subtitle: 'Q3 in twelve minutes',
    theme: 'midnight',
    slides: [
      { id: 'slide-a', layout: 'title', title: 'Quarterly review', subtitle: 'Q3 in twelve minutes', blocks: [] },
      {
        id: 'slide-b',
        layout: 'content',
        title: 'Revenue grew 24% on the back of enterprise deals',
        blocks: [
          {
            id: 'block-1',
            type: 'bullets',
            items: ['Enterprise ARR up 31%', 'Churn down to 2.1%', 'Two new regions live'],
          },
          { id: 'block-2', type: 'text', text: 'The **mid-market** segment stayed flat.' },
        ],
        notes: 'Pause on the churn number.',
      },
      {
        id: 'slide-c',
        layout: 'two-column',
        title: 'Two numbers matter',
        blocks: [
          { id: 'block-3', type: 'metric', value: '$4.2M', label: 'Net new ARR', column: 'left' },
          { id: 'block-4', type: 'metric', value: '118%', label: 'Net revenue retention', column: 'right' },
        ],
      },
      {
        id: 'slide-d',
        layout: 'statement',
        blocks: [
          { id: 'block-5', type: 'quote', text: 'Ship the boring thing first.', attribution: 'Engineering handbook' },
        ],
      },
      {
        id: 'slide-e',
        layout: 'content',
        title: 'How the pipeline works',
        blocks: [
          {
            id: 'block-6',
            type: 'code',
            code: 'const deck = await slides.create({ markdown })\nawait slides.export({ format: "pdf" })',
            language: 'ts',
          },
          {
            id: 'block-7',
            type: 'image',
            src: 'https://example.invalid/missing.png',
            alt: 'diagram',
            caption: 'Pipeline',
          },
        ],
      },
    ],
    revision: 3,
    created_at_ms: 1,
    updated_at_ms: 2,
  }
}
