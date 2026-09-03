import { createContext, type ReactNode, useContext } from 'react'
import {
  hashForWorkersConfiguration,
  hashForWorkersConfigurationList,
} from '@/hooks/use-hash-route'
import { resolveConfigurationFamily } from '@/lib/configuration-family'
import {
  type ConfigurationSchemaView,
  listConfigurations,
} from '@/pages/Configuration/tabs/WorkersTab/api'

/** Host-owned metadata for the configuration associated with an injected page. */
const PaneConfigurationContext = createContext<string | undefined>(undefined)

export function PaneConfigurationProvider({
  configurationId,
  children,
}: {
  configurationId?: string
  children: ReactNode
}) {
  return (
    <PaneConfigurationContext.Provider value={configurationId}>
      {children}
    </PaneConfigurationContext.Provider>
  )
}

export function usePaneConfigurationId(): string | undefined {
  return useContext(PaneConfigurationContext)
}

/**
 * Open the settings entry owned by a pane's stable form family.
 *
 * Dynamic III_CONFIG_NAME registrations advertise their family through
 * metadata.ui_form. If the family has no live entry, the catalog is the safest
 * recovery surface. The same is true when several named instances match: the
 * operator chooses one rather than the Console editing an arbitrary worker.
 */
export async function openPaneConfiguration(
  familyId: string,
  loadConfigurations: () => Promise<
    ConfigurationSchemaView[]
  > = listConfigurations,
): Promise<void> {
  window.location.hash = await paneConfigurationHash(
    familyId,
    loadConfigurations,
  )
}

/** Pure/testable destination selection used by the pane action. */
export async function paneConfigurationHash(
  familyId: string,
  loadConfigurations: () => Promise<ConfigurationSchemaView[]>,
): Promise<string> {
  try {
    const resolution = resolveConfigurationFamily(
      familyId,
      await loadConfigurations(),
    )
    return resolution.kind === 'resolved'
      ? hashForWorkersConfiguration(resolution.id)
      : hashForWorkersConfigurationList()
  } catch {
    return hashForWorkersConfigurationList()
  }
}
