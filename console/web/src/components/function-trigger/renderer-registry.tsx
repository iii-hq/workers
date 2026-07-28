/**
 * Ordered renderer registry for `FunctionTriggerCard` — replaces the
 * hand-chained `??` dispatch across the first-party ToolView families,
 * and prepends runtime-injected renderers (injectable UI's
 * `host.functionTriggers.register`).
 *
 * Dispatch: injected first (registration order), then first-party, then the
 * JSON fallback in the card itself. First non-null wins; `null` means
 * "fall through".
 */

import { useMemo } from 'react'
import { CoderFunctionIdLabel, CoderToolView } from '@/components/chat/coder'
import { EngineFunctionIdLabel, EngineToolView } from '@/components/chat/engine'
import { FpFunctionIdLabel, FpToolView } from '@/components/chat/fp'
import {
  HarnessFunctionIdLabel,
  HarnessToolView,
} from '@/components/chat/harness'
import { RouterFunctionIdLabel, RouterToolView } from '@/components/chat/router'
import {
  SandboxFunctionIdLabel,
  SandboxToolView,
} from '@/components/chat/sandbox'
import {
  ScraplingFunctionIdLabel,
  ScraplingToolView,
} from '@/components/chat/scrapling'
import { ShellFunctionIdLabel, ShellToolView } from '@/components/chat/shell'
import { StateFunctionIdLabel, StateToolView } from '@/components/chat/state'
import { WebFunctionIdLabel, WebToolView } from '@/components/chat/web'
import { WorkerFunctionIdLabel, WorkerToolView } from '@/components/chat/worker'
import {
  WorkflowFunctionIdLabel,
  WorkflowToolView,
} from '@/components/chat/workflow'
import { ExtErrorChip, ScopedExtension } from '@/lib/ui-loader'
import { type RegisteredRenderer, useExtRenderers } from '@/lib/ui-slots'
import type { FunctionTriggerMessage } from '@/types/chat'
import type { FunctionTriggerRenderer } from '@/types/injectable-ui'

/**
 * The first-party families (11 since directory and browser moved into their
 * workers' injected UI), in the exact order of the old `??` chains. Each
 * family's `tryRender*` already gates on its own function ids, so an entry
 * returning `null` falls through to the next.
 */
export const FIRST_PARTY_RENDERERS: readonly FunctionTriggerRenderer[] = [
  {
    id: 'first-party/sandbox',
    isMatch: SandboxToolView.isSandboxFunction,
    tryRender: SandboxToolView.tryRender,
    tryRenderRunning: SandboxToolView.tryRenderRunning,
    tryRenderPreview: SandboxToolView.tryRenderPreview,
    FunctionIdLabel: SandboxFunctionIdLabel,
  },
  {
    id: 'first-party/engine',
    isMatch: EngineToolView.isEngineListFunction,
    tryRender: EngineToolView.tryRender,
    tryRenderRunning: EngineToolView.tryRenderRunning,
    tryRenderPreview: EngineToolView.tryRenderPreview,
    FunctionIdLabel: EngineFunctionIdLabel,
  },
  // directory::* rendering is no longer first-party: the iii-directory
  // worker ships it as injected UI (iii-directory/ui/src/function-trigger).
  {
    id: 'first-party/worker',
    isMatch: WorkerToolView.isWorkerFunction,
    tryRender: WorkerToolView.tryRender,
    tryRenderRunning: WorkerToolView.tryRenderRunning,
    tryRenderPreview: WorkerToolView.tryRenderPreview,
    FunctionIdLabel: WorkerFunctionIdLabel,
  },
  {
    id: 'first-party/web',
    isMatch: WebToolView.isWebFunction,
    tryRender: WebToolView.tryRender,
    tryRenderRunning: WebToolView.tryRenderRunning,
    tryRenderPreview: WebToolView.tryRenderPreview,
    FunctionIdLabel: WebFunctionIdLabel,
  },
  {
    id: 'first-party/coder',
    isMatch: CoderToolView.isCoderFunction,
    tryRender: CoderToolView.tryRender,
    tryRenderRunning: CoderToolView.tryRenderRunning,
    tryRenderPreview: CoderToolView.tryRenderPreview,
    FunctionIdLabel: CoderFunctionIdLabel,
  },
  {
    id: 'first-party/scrapling',
    isMatch: ScraplingToolView.isScraplingFunction,
    tryRender: ScraplingToolView.tryRender,
    tryRenderRunning: ScraplingToolView.tryRenderRunning,
    tryRenderPreview: ScraplingToolView.tryRenderPreview,
    FunctionIdLabel: ScraplingFunctionIdLabel,
  },
  {
    id: 'first-party/shell',
    isMatch: ShellToolView.isShellFunction,
    tryRender: ShellToolView.tryRender,
    tryRenderRunning: ShellToolView.tryRenderRunning,
    tryRenderPreview: ShellToolView.tryRenderPreview,
    FunctionIdLabel: ShellFunctionIdLabel,
  },
  {
    id: 'first-party/workflow',
    isMatch: WorkflowToolView.isWorkflowFunction,
    tryRender: WorkflowToolView.tryRender,
    tryRenderRunning: WorkflowToolView.tryRenderRunning,
    tryRenderPreview: WorkflowToolView.tryRenderPreview,
    FunctionIdLabel: WorkflowFunctionIdLabel,
  },
  {
    id: 'first-party/router',
    isMatch: RouterToolView.isRouterFunction,
    tryRender: RouterToolView.tryRender,
    tryRenderRunning: RouterToolView.tryRenderRunning,
    tryRenderPreview: RouterToolView.tryRenderPreview,
    FunctionIdLabel: RouterFunctionIdLabel,
  },
  {
    id: 'first-party/harness',
    isMatch: HarnessToolView.isHarnessFunction,
    tryRender: HarnessToolView.tryRender,
    tryRenderRunning: HarnessToolView.tryRenderRunning,
    tryRenderPreview: HarnessToolView.tryRenderPreview,
    FunctionIdLabel: HarnessFunctionIdLabel,
  },
  {
    id: 'first-party/fp',
    isMatch: FpToolView.isFpFunction,
    tryRender: FpToolView.tryRender,
    tryRenderRunning: FpToolView.tryRenderRunning,
    tryRenderPreview: FpToolView.tryRenderPreview,
    FunctionIdLabel: FpFunctionIdLabel,
  },
  {
    id: 'first-party/state',
    isMatch: StateToolView.isStateFunction,
    tryRender: StateToolView.tryRender,
    tryRenderRunning: StateToolView.tryRenderRunning,
    tryRenderPreview: StateToolView.tryRenderPreview,
    FunctionIdLabel: StateFunctionIdLabel,
  },
]

/**
 * Fence one injected renderer: a throw inside `tryRender*` (called during
 * the host card's render, outside any boundary) degrades to a chip, and the
 * returned node mounts inside the script's scope wrapper + ErrorBoundary.
 */
function fenceInjected(entry: RegisteredRenderer): FunctionTriggerRenderer {
  const { renderer, scope, path } = entry
  const wrap = (
    render: (message: FunctionTriggerMessage) => React.ReactNode | null,
  ) => {
    return (message: FunctionTriggerMessage): React.ReactNode | null => {
      let node: React.ReactNode | null = null
      try {
        node = render(message)
      } catch (error) {
        console.error(`[iii-ui] renderer ${renderer.id} threw`, error)
        return (
          <ExtErrorChip
            path={path}
            error={error instanceof Error ? error : new Error(String(error))}
          />
        )
      }
      if (node == null) return null
      return (
        <ScopedExtension scope={scope} path={path}>
          {node}
        </ScopedExtension>
      )
    }
  }
  return {
    id: renderer.id,
    isMatch: (functionId) => {
      try {
        return renderer.isMatch(functionId)
      } catch {
        return false
      }
    },
    tryRender: wrap((m) => renderer.tryRender(m)),
    tryRenderRunning: renderer.tryRenderRunning
      ? wrap((m) => renderer.tryRenderRunning?.(m) ?? null)
      : undefined,
    tryRenderPreview: renderer.tryRenderPreview
      ? wrap((m) => renderer.tryRenderPreview?.(m) ?? null)
      : undefined,
    FunctionIdLabel: renderer.FunctionIdLabel,
    primaryTabLabel: renderer.primaryTabLabel,
  }
}

/**
 * The live dispatch order: injected renderers (fenced) first, then the
 * first-party families. Re-computes exactly when a script (re)registers.
 */
export function useFunctionTriggerRenderers(): readonly FunctionTriggerRenderer[] {
  const injected = useExtRenderers()
  return useMemo(
    () => [...injected.map(fenceInjected), ...FIRST_PARTY_RENDERERS],
    [injected],
  )
}

export function firstNonNull(
  renderers: readonly FunctionTriggerRenderer[],
  pick: (renderer: FunctionTriggerRenderer) => React.ReactNode | null,
): React.ReactNode | null {
  for (const renderer of renderers) {
    const node = pick(renderer)
    if (node != null) return node
  }
  return null
}
