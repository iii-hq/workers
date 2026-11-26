import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  type ExtensionIii,
  type StatusDotProps,
  StatusPanel,
} from '@iii-dev/console-ui';
import { type RefObject, useEffect, useRef, useState } from 'react';

export type Status = 'busy' | 'idle' | 'waiting' | 'done' | 'error' | 'stopped' | 'unknown';

export const STATUS: Record<
  Status,
  { word: string; tone: NonNullable<StatusDotProps['tone']>; pulse?: boolean }
> = {
  busy: { word: 'Working', tone: 'accent', pulse: true },
  waiting: { word: 'Waiting', tone: 'warn' },
  idle: { word: 'Idle', tone: 'ghost' },
  done: { word: 'Done', tone: 'ok' },
  error: { word: 'Failed', tone: 'alert' },
  stopped: { word: 'Stopped', tone: 'ghost' },
  unknown: { word: 'Unknown', tone: 'ghost' },
};

type ReplySignal = { claude_session_id: string };
type Snapshot = {
  claude_session_id: string;
  name: string | null;
  status: Status;
  last_reply: { id: string; text: string } | null;
};
type View = { state: 'loading' | 'unavailable' } | { state: 'ready'; data: Snapshot };

function Follow({ iii, id }: { iii: ExtensionIii; id: string }) {
  const [view, setView] = useState<View>({ state: 'loading' });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    // Unique per mount: a delivery already queued for a previous selection must
    // not address the new viewer, even when the same conversation is reopened.
    const base = `iii::claude-ui::follow-${crypto.randomUUID()}`;
    let active = true;
    let reading = false;
    let again = false;
    const cleanup: (() => void)[] = [];
    const stop = () => {
      active = false;
      for (const off of cleanup.splice(0).reverse()) off();
    };
    const unavailable = () => {
      if (!active) return;
      stop();
      setView({ state: 'unavailable' });
    };
    const read = async () => {
      if (!active) return;
      if (reading) {
        again = true;
        return;
      }
      reading = true;
      try {
        do {
          again = false;
          const data = await iii.call<Snapshot>(
            'claude::agents::get',
            { claude_session_id: id },
            { timeoutMs: 30000 },
          );
          if (data.claude_session_id !== id) throw new Error('Session changed');
          if (active && !again) setView({ state: 'ready', data });
        } while (active && again);
      } catch {
        unavailable();
      } finally {
        reading = false;
      }
    };
    setView({ state: 'loading' });
    try {
      cleanup.push(
        iii.on<ReplySignal | { output: ReplySignal }>(base, (delivery) => {
          // Custom deliveries carry captured output; ADE may also project it.
          const event = delivery && 'output' in delivery ? delivery.output : delivery;
          // Treat delivery only as a change signal. A queued reply cannot bypass
          // the current permission check, or overwrite a newer lookup with old text.
          if (event?.claude_session_id === id) void read();
        }),
      );
      cleanup.push(iii.on(`${base}-config`, unavailable));
      cleanup.push(iii.observe('claude::on-config-change', `${base}-config::${iii.browserId}`));
      cleanup.push(
        iii.observe(
          { function_id: 'claude::agents::on-reply', args: { claude_session_id: id } },
          `${base}::${iii.browserId}`,
        ),
      );
      cleanup.push(
        iii.addConnectionStateListener((state) => {
          if (state !== 'connected') unavailable();
        }),
      );
      // The host doesn't surface asynchronous observer admission or gaps. Bound
      // stale state even if no event arrives; only this open dialog does a read.
      const timer = window.setInterval(() => void read(), 30000);
      cleanup.push(() => window.clearInterval(timer));
      void read();
    } catch {
      unavailable();
    }
    return stop;
  }, [iii, id, attempt]);

  if (view.state === 'unavailable')
    return (
      <StatusPanel
        role="alert"
        variant="warn"
        headline="Replies unavailable"
        detail="Access or the session source changed. Check access, then try again."
        action={
          <Button size="sm" variant="ghost" onClick={() => setAttempt((n) => n + 1)}>
            Retry
          </Button>
        }
      />
    );
  if (view.state !== 'ready') return <p role="status">Checking reply access…</p>;
  const { data } = view;
  return (
    <div className="claude-follow-content">
      <p className="claude-follow-state">
        {data.name ?? id.slice(0, 8)} · {(STATUS[data.status] ?? STATUS.unknown).word}
      </p>
      <h3>Latest assistant reply</h3>
      <p aria-live="polite" className="claude-follow-text">
        {data.last_reply?.text ?? 'No assistant reply yet.'}
      </p>
    </div>
  );
}

/** A modal keeps keystrokes out of the unrelated terminal behind the viewer. */
export function SessionDetail({
  iii,
  id,
  available,
  opener,
  onClose,
}: {
  iii: ExtensionIii;
  id: string;
  available: boolean;
  opener: RefObject<HTMLButtonElement | null>;
  onClose: () => void;
}) {
  const close = useRef<HTMLButtonElement>(null);
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent showCloseButton={false} initialFocus={close} finalFocus={opener}>
        <DialogTitle>Follow original</DialogTitle>
        <DialogDescription>
          Read only. This session has no terminal you can attach to here. Use its original terminal
          to send input.
        </DialogDescription>
        {available ? (
          <Follow iii={iii} id={id} />
        ) : (
          <StatusPanel
            role="alert"
            variant="warn"
            headline="Session unavailable"
            detail="Close this view and refresh the session list."
          />
        )}
        <div className="claude-follow-actions">
          <Button ref={close} size="sm" variant="secondary" onClick={onClose}>
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
