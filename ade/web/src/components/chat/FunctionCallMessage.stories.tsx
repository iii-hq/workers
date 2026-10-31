import type { Meta, StoryObj } from '@storybook/react-vite'
import { useEffect } from 'react'
import { SpawnActivityCard } from '@/components/chat/harness/SpawnView'
import { FunctionCallCard } from '@/components/function-call/FunctionCallCard'
import { registerExtRenderer } from '@/lib/ui-slots'
import { coderFixtures } from '@/stories/fixtures/coder-fixtures'
import { directoryFixtures } from '@/stories/fixtures/directory-fixtures'
import {
  engineFixtures,
  harnessObserveCallGuarded,
  harnessObserveFailed,
  harnessObserveRunning,
  harnessObserveState,
} from '@/stories/fixtures/engine-fixtures'
import { fpFixtures } from '@/stories/fixtures/fp-fixtures'
import {
  harnessFixtures,
  spawnDirectDone,
} from '@/stories/fixtures/harness-fixtures'
import { routerFixtures } from '@/stories/fixtures/router-fixtures'
import { sandboxFixtures } from '@/stories/fixtures/sandbox-fixtures'
import { shellFixtures } from '@/stories/fixtures/shell-fixtures'
import { stateFixtures } from '@/stories/fixtures/state-fixtures'
import { webFixtures } from '@/stories/fixtures/web-fixtures'
import { workerFixtures } from '@/stories/fixtures/worker-fixtures'
import { workflowFixtures } from '@/stories/fixtures/workflow-fixtures'
import { worktreeFixtures } from '@/stories/fixtures/worktree-fixtures'
import type { FunctionCallMessage as FCallType } from '@/types/chat'
import { FileChangesCard } from '../../../../../ide/ui/src/function-call/FileChangesCard'
import { summarizeFileChanges } from '../../../../../ide/ui/src/function-call/file-changes'
import '../../../../../ide/ui/styles.css'

const fcallPendingSingle: FCallType = {
  id: 'f0a',
  role: 'function-call',
  functionId: 'engine::echo',
  input: { text: 'hello, world.' },
  pendingApproval: true,
  createdAt: Date.now(),
}

const fcallPendingMulti: FCallType = {
  id: 'f0b',
  role: 'function-call',
  functionId: 'fs::write',
  input: {
    path: '/tmp/scratch/notes.md',
    contents: '# scratch\n\njust a draft.\n',
    overwrite: true,
  },
  pendingApproval: true,
  createdAt: Date.now(),
}

const fcallRunning: FCallType = {
  id: 'f1',
  role: 'function-call',
  functionId: 'engine::echo',
  description: 'Checking the service response',
  input: { text: 'hello, world.' },
  running: true,
  createdAt: Date.now(),
}

const fcallDone: FCallType = {
  id: 'f2',
  role: 'function-call',
  functionId: 'engine::echo',
  input: { text: 'hello, world.' },
  output: { text: 'hello, world.' },
  durationMs: 132,
  createdAt: Date.now(),
}

const fcallActivity: FCallType = {
  ...fcallDone,
  id: 'f2-activity',
  description: 'Checking the service response',
}

const fileChangesActivity: FCallType = {
  id: 'f-files-activity',
  role: 'function-call',
  functionId: 'coder::update-file',
  description: 'Updating project files',
  input: {
    files: [
      'pnpm-workspace.yaml',
      'storage/Cargo.toml',
      'storage/README.md',
      'src/config.ts',
      'src/index.ts',
      'tests/config.test.ts',
      'docs/setup.md',
      'package.json',
    ].map((path, index) => ({
      path,
      ops: [
        {
          op: 'update_lines',
          from_line: index + 1,
          to_line: index + 1,
          content: `updated line ${index + 1}\nadded line\n`,
        },
      ],
    })),
  },
  output: {
    results: [
      'pnpm-workspace.yaml',
      'storage/Cargo.toml',
      'storage/README.md',
      'src/config.ts',
      'src/index.ts',
      'tests/config.test.ts',
      'docs/setup.md',
      'package.json',
    ].map((path, index) => ({
      path: `/repo/${path}`,
      success: true,
      applied: 1,
      change_id: `story-change-${index}`,
    })),
  },
  durationMs: 824,
  createdAt: Date.now(),
}

const fileChangesSummary = summarizeFileChanges(
  fileChangesActivity.functionId,
  fileChangesActivity.input,
  fileChangesActivity.output,
)

function FileChangesActivityStory() {
  useEffect(
    () =>
      registerExtRenderer({
        renderer: {
          id: 'shell/story#file-changes',
          isMatch: (functionId) => functionId === 'coder::update-file',
          tryRender: () =>
            fileChangesSummary ? (
              <FileChangesCard
                summary={fileChangesSummary}
                running={false}
                onOpenDiff={() => {}}
              />
            ) : null,
          metadata: { display: true },
        },
        scope: 'shell',
        path: 'shell/page.js',
      }),
    [],
  )
  return <FunctionCallCard message={fileChangesActivity} />
}

const fcallDoneMulti: FCallType = {
  id: 'f3',
  role: 'function-call',
  functionId: 'directory::index',
  input: { path: '/repo/src', recursive: true, glob: '*.ts' },
  output: { files: 142, bytes: 318_204, elapsedMs: 410 },
  durationMs: 410,
  createdAt: Date.now(),
}

/** Denied at the approval gate: the gate's DenialEnvelope rides in the error
 *  details — the call never ran, so the header makes no "called" claim. */
const fcallDenied: FCallType = {
  id: 'f3b',
  role: 'function-call',
  functionId: 'shell::run',
  input: { command: 'rm -rf /tmp/cache' },
  output: {
    error: {
      kind: 'function_error',
      message: 'Rejected by operator.',
      details: {
        schema_version: 1,
        status: 'denied',
        denied_by: 'user',
        function_id: 'shell::run',
        reason: 'Rejected by operator.',
      },
    },
  },
  createdAt: Date.now(),
}

/** TracesV2 sub-span that inherited its function id from the enclosing
 *  invocation: its 3ms measure machinery inside the call, so the header
 *  stays verb-less — no "called ƒ …" claim. */
const fcallInheritedIdentity: FCallType = {
  id: 'f3c',
  role: 'function-call',
  functionId: 'directory::index',
  input: { path: '/repo/src' },
  output: { rows: 12 },
  durationMs: 3,
  identityInherited: true,
  createdAt: Date.now(),
}

const fcallError: FCallType = {
  id: 'f4',
  role: 'function-call',
  functionId: 'engine::functions::info',
  input: { function_id: 'nope::missing' },
  output: {
    error: {
      kind: 'function_error',
      message: "Function 'nope::missing' is not registered.",
    },
  },
  durationMs: 87,
  createdAt: Date.now(),
}

/** Bordered gallery of every fixture in a namespace family, all expanded so
 *  the custom terminal/preview panes render (not just the collapsed header). */
function FamilyGallery({ fixtures }: { fixtures: readonly FCallType[] }) {
  return (
    <div className="grid grid-cols-1 @3xl:grid-cols-2 gap-6 @container">
      {fixtures.map((fixture) => (
        <div key={fixture.id} className="border border-rule bg-bg">
          <div className="bg-panel px-3 py-1.5 border-b border-rule-2 iii-ui-eyebrow">
            {fixture.functionId}
          </div>
          <div className="p-4">
            <FunctionCallCard message={fixture} defaultOpen />
          </div>
        </div>
      ))}
    </div>
  )
}

const meta = {
  title: 'Chat/FunctionCallMessage',
  component: FunctionCallCard,
  parameters: { layout: 'padded' },
  // Default `message` so the render-only family galleries don't have to
  // declare it; the per-state stories below override it via their own args.
  args: { message: fcallDone },
} satisfies Meta<typeof FunctionCallCard>

export default meta
type Story = StoryObj<typeof meta>

export const PendingSingleArg: Story = {
  name: 'awaiting permission (single arg)',
  args: { message: fcallPendingSingle },
}

export const PendingMultiArg: Story = {
  name: 'awaiting permission (multi arg)',
  args: { message: fcallPendingMulti },
}

export const Running: Story = {
  name: 'calling',
  args: { message: fcallRunning },
}

export const DoneCollapsed: Story = {
  name: 'called (collapsed)',
  args: { message: fcallDone },
}

export const DescribedActivity: Story = {
  name: 'agent activity (collapsed)',
  args: { message: fcallActivity },
}

export const FileChangesArtifact: Story = {
  name: 'agent activity · file changes artifact',
  render: () => <FileChangesActivityStory />,
}

export const SpawnedSubagentWidget: Story = {
  name: 'spawned sub-agent widget',
  args: { message: spawnDirectDone },
}

export const SubagentLiveStates: Story = {
  name: 'sub-agent live states',
  render: () => {
    const now = Date.now()
    return (
      <div className="grid max-w-5xl gap-3">
        {(
          [
            ['queued', 'Queued'],
            ['waiting', 'Waiting'],
            ['working', 'Working'],
            ['thinking', 'Thinking'],
            ['messaging', 'Sending a message'],
            ['ended', 'Ended'],
            ['stopped', 'Stopped'],
            ['error', 'Needs attention'],
            ['disconnected', 'Disconnected'],
          ] as const
        ).map(([status, title]) => (
          <SpawnActivityCard
            key={status}
            title={title}
            task="Audit the release candidate and report any blocker."
            status={status}
            sessionId={`sub_${status}_8f3a7d2e`}
            createdAt={now - 120_000}
            activityAt={now - 120_000}
            now={now}
            onOpen={() => {}}
          />
        ))}
      </div>
    )
  },
}

export const ObserverAddedWidget: Story = {
  name: 'observer added widget',
  args: { message: harnessObserveState },
}

export const ObserverAddedDetails: Story = {
  name: 'observer added details (call, conditions, lifecycle)',
  args: { message: harnessObserveCallGuarded, defaultOpen: true },
}

export const ObserverAdding: Story = {
  name: 'observer adding (running)',
  args: { message: harnessObserveRunning, defaultOpen: true },
}

export const ObserverNotAdded: Story = {
  name: 'observer not added',
  args: { message: harnessObserveFailed, defaultOpen: true },
}

export const DoneExpanded: Story = {
  name: 'called · single-field (expanded)',
  args: { message: fcallDone, defaultOpen: true },
}

export const DoneMultiFieldExpanded: Story = {
  name: 'called · multi-field (expanded)',
  args: { message: fcallDoneMulti, defaultOpen: true },
}

export const DeniedNeverRan: Story = {
  name: 'denied (never ran)',
  args: { message: fcallDenied, defaultOpen: true },
}

export const InheritedIdentity: Story = {
  name: 'inherited identity (traces sub-span)',
  args: { message: fcallInheritedIdentity },
}

export const ErrorCollapsed: Story = {
  name: 'error (collapsed)',
  args: { message: fcallError },
}

export const ErrorExpanded: Story = {
  name: 'error (expanded)',
  args: { message: fcallError, defaultOpen: true },
}

export const SandboxFamily: Story = {
  name: 'sandbox family',
  render: () => <FamilyGallery fixtures={sandboxFixtures} />,
}

export const ShellFamily: Story = {
  name: 'shell family',
  render: () => <FamilyGallery fixtures={shellFixtures} />,
}

export const DirectoryFamily: Story = {
  name: 'directory family',
  render: () => <FamilyGallery fixtures={directoryFixtures} />,
}

export const EngineFamily: Story = {
  name: 'engine family',
  render: () => <FamilyGallery fixtures={engineFixtures} />,
}

export const WebFamily: Story = {
  name: 'web family',
  render: () => <FamilyGallery fixtures={webFixtures} />,
}

export const WorkerFamily: Story = {
  name: 'worker family',
  render: () => <FamilyGallery fixtures={workerFixtures} />,
}

export const CoderFamily: Story = {
  name: 'coder family',
  render: () => <FamilyGallery fixtures={coderFixtures} />,
}

export const WorkflowFamily: Story = {
  name: 'workflow family',
  render: () => <FamilyGallery fixtures={workflowFixtures} />,
}

export const WorktreeFamily: Story = {
  name: 'worktree family',
  render: () => <FamilyGallery fixtures={worktreeFixtures} />,
}

export const RouterFamily: Story = {
  name: 'router family',
  render: () => <FamilyGallery fixtures={routerFixtures} />,
}

export const HarnessFamily: Story = {
  name: 'harness family',
  render: () => <FamilyGallery fixtures={harnessFixtures} />,
}

export const FpFamily: Story = {
  name: 'fp family',
  render: () => <FamilyGallery fixtures={fpFixtures} />,
}

export const StateFamily: Story = {
  name: 'state family',
  render: () => <FamilyGallery fixtures={stateFixtures} />,
}
