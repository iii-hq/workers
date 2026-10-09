/**
 * Screen E — a file the user edited that the kit also changed. The console
 * diff is two-sided, so the three-way merge is shown as three two-way
 * diffs: the kit's changes (base → kit), yours (base → yours), and the
 * result (yours → merged) whose right side is editable and starts from the
 * worker's automatic merge, with conflict markers where it could not merge.
 */

import { FileDiff, SegmentedControl, StatusPanel, Tabs, TabsContent, TabsList, TabsTrigger } from '@iii-dev/console-ui'
import { TriangleAlert } from 'lucide-react'
import { useRef, useState } from 'react'
import type { PlanRecord } from './api'
import { type Choices, decisionLabel, effectiveDecision, hasConflictMarkers } from './model'
import type { Decision, Plan, PlanFile } from './types'

type Tab = 'kit' | 'yours' | 'result'

export function ConflictView({
  plan,
  record,
  file,
  choices,
  onChoose,
}: {
  plan: Plan
  record: PlanRecord
  file: PlanFile
  choices: Choices
  onChoose: (decision: Decision, content?: string) => void
}) {
  const [tab, setTab] = useState<Tab>(file.merge === 'conflicts' ? 'result' : 'kit')
  const [style, setStyle] = useState<'split' | 'unified'>('split')
  const base = (file.base && record.contents?.blobs[file.base]) ?? ''
  const theirs = (file.theirs && record.contents?.blobs[file.theirs]) ?? ''
  const ours = record.contents?.ours[file.path] ?? ''
  const merged = record.contents?.merged[file.path] ?? ours
  // The editable side starts from the merge (or the user's earlier edit) and
  // stays stable while they type: edits flow out through onChoose only.
  const initial = useRef(choices[file.path]?.content ?? merged)
  const decision = effectiveDecision(file, choices)
  const current = choices[file.path]?.content ?? merged
  const markers = hasConflictMarkers(current)
  const from = plan.from ?? 'installed'
  const to = plan.to ?? 'new'
  const name = file.path.split('/').pop() ?? file.path

  return (
    <div className="dir-ui-kit-conflict">
      <div className="dir-ui-kit-conflict-head">
        <TriangleAlert aria-hidden />
        <span>
          {file.merge === 'conflicts'
            ? `Edited by you and changed in the kit — ${file.conflicts ?? 1} conflict${(file.conflicts ?? 1) === 1 ? '' : 's'}`
            : 'Edited by you and changed in the kit — merges cleanly'}
        </span>
      </div>
      <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)} className="dir-ui-kit-tabs">
        <div className="dir-ui-kit-tabs-bar">
          <TabsList variant="line">
            <TabsTrigger value="kit" icon={false}>
              Kit's changes
            </TabsTrigger>
            <TabsTrigger value="yours" icon={false}>
              Your changes
            </TabsTrigger>
            <TabsTrigger value="result" icon={false}>
              Result
            </TabsTrigger>
          </TabsList>
          <SegmentedControl<'split' | 'unified'>
            value={style}
            onChange={setStyle}
            options={[
              { value: 'split', label: 'Split', icon: false },
              { value: 'unified', label: 'Unified', icon: false },
            ]}
            aria-label="Diff layout"
          />
        </div>
        <TabsContent value="kit" className="dir-ui-kit-tab">
          <p className="dir-ui-kit-fine">
            What the kit changed between {from} and {to}.
          </p>
          <FileDiff
            oldFile={{ name: `${from}/${name}`, contents: base }}
            newFile={{ name: `${to}/${name}`, contents: theirs }}
            diffStyle={style}
          />
        </TabsContent>
        <TabsContent value="yours" className="dir-ui-kit-tab">
          <p className="dir-ui-kit-fine">What you changed since {from} was installed.</p>
          <FileDiff
            oldFile={{ name: `${from}/${name}`, contents: base }}
            newFile={{ name: `yours/${name}`, contents: ours }}
            diffStyle={style}
          />
        </TabsContent>
        <TabsContent value="result" className="dir-ui-kit-tab" forceMount>
          <p className="dir-ui-kit-fine">
            Your file on the left, the result on the right — edit it there. It starts from the automatic merge; resolve
            every <code>{'<<<<<<<'}</code> block before using it.
          </p>
          <FileDiff
            oldFile={{ name: `yours/${name}`, contents: ours }}
            newFile={{ name: `result/${name}`, contents: initial.current }}
            diffStyle={style}
            edit
            onChange={(text) => onChoose('merged', text)}
          />
        </TabsContent>
      </Tabs>
      <div className="dir-ui-kit-conflict-choice">
        <SegmentedControl<Decision>
          variant="radio"
          // No decision yet: nothing is selected (not even the merge).
          value={(decision ?? '') as Decision}
          onChange={(d) => (d === 'merged' ? onChoose('merged', current) : onChoose(d))}
          options={file.options.map((d) => ({ value: d, label: decisionLabel(d, file), icon: false }))}
          className="dir-ui-kit-radio"
          aria-label={`Resolve ${file.path}`}
        />
        {decision === null ? <span className="dir-ui-kit-fine">Choose how to resolve this file to update.</span> : null}
      </div>
      {decision === 'merged' && markers ? (
        <StatusPanel
          variant="warn"
          headline="The result still has conflict markers."
          detail="Edit the Result tab until no <<<<<<< block remains, or pick the kit's version or yours."
        />
      ) : null}
    </div>
  )
}
