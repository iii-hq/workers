import { SequencePlayer } from '@/components/diagrams/SequencePlayer'
import { Section } from '@/components/Section'
import { StatusPanel } from '@/components/schematic/StatusPanel'
import { SEQ_LANES, SEQ_STEPS } from '@/content/sequence'

/**
 * A5 - the source of truth. Steps through the generation lifecycle to make the
 * load-bearing point: the schema codegen needs already exists in the engine, so
 * codegen is a projection, not a parser of your code. Closes with the honest
 * limit (live catalog only).
 */
export function SourceOfTruthSection() {
  return (
    <Section
      id="discover"
      index="04"
      eyebrow="the source of truth"
      title="iii already describes every function in json schema."
      lede="each worker's macro emits a request and response schema at startup, and the engine hands it back through engine::*::info. codegen reads that, maps it, and emits. it never parses your source or guesses a shape."
    >
      <SequencePlayer
        title="generate, step by step"
        lanes={SEQ_LANES}
        steps={SEQ_STEPS}
      />

      <div className="mt-6">
        <StatusPanel
          variant="info"
          headline="the catalog is live"
          detail="codegen sees the workers connected to the engine right now, so a glob that matches nothing is a warning, not an error. a checked-in catalog snapshot, so ci needn't boot every worker, is the main planned v2 addition; a go emitter is reserved."
        />
      </div>
    </Section>
  )
}
