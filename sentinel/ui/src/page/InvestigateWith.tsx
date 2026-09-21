import {
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Selector,
} from '@iii-dev/console-ui'
import type { Host } from '@iii-dev/console-ui'
import { useState } from 'react'
import { modelGroups, splitKey } from '../settings/catalog.js'
import { useModelCatalog } from '../settings/useModelCatalog'

interface Props {
  host: Host
  onCancel: () => void
  onInvestigate: (model: string, provider: string) => void
  open: boolean
}

/**
 * Picking the model for one investigation, without changing the default.
 *
 * This is the only dialog the page has, and it exists for the same reason
 * the picker in the settings form does: a model id typed by hand is a typo
 * that surfaces as a failed turn. It opens on whatever is configured, so the
 * common case is one more click rather than a decision.
 */
export function InvestigateWith({ host, onCancel, onInvestigate, open }: Props) {
  const { catalog, loading } = useModelCatalog(host)
  // Opens empty on purpose: somebody who chose this menu item came to pick.
  // Plain Investigate is the one that uses the configured default.
  const [selected, setSelected] = useState('')

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : onCancel())}>
      <DialogContent>
        <DialogTitle>Investigate with…</DialogTitle>
        <DialogDescription>
          For this investigation only. The configured default is unchanged.
        </DialogDescription>
        <Selector
          aria-label="Model for this investigation"
          value={selected || undefined}
          groups={modelGroups(catalog, selected)}
          loading={loading}
          placeholder="pick a model"
          searchPlaceholder="model or provider"
          emptyMessage="The router is serving no models. Configure a provider first."
          onCreate={(query) => setSelected(query.trim())}
          createOptionLabel={(query) => `use ${query}`}
          onChange={setSelected}
        />
        <div className="sentinel-ui-dialog-actions">
          <DialogClose asChild>
            <Button variant="ghost" size="sm" onClick={onCancel}>
              Cancel
            </Button>
          </DialogClose>
          <Button
            size="sm"
            disabled={!selected}
            onClick={() => {
              const { model, provider } = splitKey(selected)
              if (model) onInvestigate(model, provider)
            }}
          >
            Investigate
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
