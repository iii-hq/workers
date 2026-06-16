import { SpawnTree } from '@/components/diagrams/SpawnTree'
import { Section } from '@/components/Section'
import { SpecSheet, SpecRow } from '@/components/SpecSheet'
import { CodeBlock, K, M, S, C } from '@/components/schematic/CodeBlock'

export function SpawnSection() {
  return (
    <Section
      id="spawn"
      index="07"
      eyebrow="sub-agents"
      title="spawn is one function call — children are ordinary sessions, parallel by construction."
      lede="no orchestrator process, no second framework: a parent turn fans out bounded children and joins on their typed results, all on the same durable queue and the same reactive surface as everything else."
    >
      <SpawnTree />

      <div className="mt-6 grid grid-cols-1 @4xl:grid-cols-2 gap-4 items-start">
        <CodeBlock title="what the model actually emits">
          <K>agent_trigger</K> <M>{'{'}</M>
          {'\n'}
          {'  '}function: <S>"harness::spawn"</S>,{'\n'}
          {'  '}payload: <M>{'{'}</M>
          {'\n'}
          {'    '}task: <S>"compare pricing + limits across providers"</S>,{'\n'}
          {'    '}options: <M>{'{'}</M>
          {'\n'}
          {'      '}output: <M>{'{'}</M> type: <S>"json"</S>, schema:{' '}
          <M>{'{ … }'}</M> <M>{'}'}</M>,{'\n'}
          {'      '}functions: <M>{'{'}</M> allow: [<S>"search::web"</S>,{' '}
          <S>"router::models::list"</S>] <M>{'}'}</M>
          {'\n'}
          {'    '}
          <M>{'}'}</M>
          {'\n'}
          {'  '}
          <M>{'}'}</M>
          {'\n'}
          <M>{'}'}</M>
          {'\n\n'}
          <C>// the child's allow-list is intersected with the parent's —</C>
          {'\n'}
          <C>// narrow, never escalate. linkage is injected by the harness,</C>
          {'\n'}
          <C>// never trusted from model arguments.</C>
        </CodeBlock>

        <SpecSheet title="the spawn dispatch, step by step" meta="5 moves" defaultOpen>
          <div className="flex flex-col">
            <SpecRow name="1 — guards" type="fail as results, not throws">
              spawn must itself be allowed; depth and fan-out caps refuse
              politely — the model reads the error and adapts.
            </SpecRow>
            <SpecRow name="2 — policy subsetting" type="parent ∩ request">
              children can narrow their reach, never widen it; turn budgets
              cap at the parent's remainder.
            </SpecRow>
            <SpecRow name="3 — child session created" type="linkage in metadata">
              parent_session_id / parent_turn_id / call id / depth — any ui
              rebuilds the tree, and event filters can target one branch.
            </SpecRow>
            <SpecRow name="4 — parent parks" type="pending call">
              the same durable-execution primitive as approvals — no queue
              step held while children work.
            </SpecRow>
            <SpecRow name="5 — completion resolves" type="typed join">
              completed delivers the contract result; failed or cancelled
              deliver an explicit error. deterministic ids make
              double-resolution impossible.
            </SpecRow>
          </div>
        </SpecSheet>
      </div>
    </Section>
  )
}
