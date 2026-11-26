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

// Web Crypto provides random bytes even on plain HTTP LAN origins.
export function sessionActionNonce(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
    byte.toString(16).padStart(2, '0'),
  ).join('');
}

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

type Reply = { id: string; text: string };
type ReplyDelivery = { claude_session_id: string; reply: Reply };
type Snapshot = {
  claude_session_id: string;
  name: string | null;
  status: Status;
  last_reply: Reply | null;
  replies: Reply[];
  cursor: string;
};
type View = { state: 'loading' | 'unavailable' } | { state: 'ready'; data: Snapshot };

function Follow({ iii, id, configId }: { iii: ExtensionIii; id: string; configId: string }) {
  const [view, setView] = useState<View>({ state: 'loading' });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    // Unique per mount: a delivery already queued for a previous selection must
    // not address the new viewer, even when the same conversation is reopened.
    const base = `iii::claude-ui::follow-${sessionActionNonce()}`;
    let active = true;
    let reading = false;
    let again = false;
    let cursor: string | undefined;
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
          const data = await iii.call<Snapshot | { retry: true }>(
            'claude::agents::follow-snapshot',
            { claude_session_id: id, ...(cursor ? { cursor } : {}) },
            { timeoutMs: 30000 },
          );
          if ('retry' in data) continue;
          if (data.claude_session_id !== id) throw new Error('Session changed');
          if (active && !again) {
            const unchanged = cursor === data.cursor;
            setView((previous) => ({
              state: 'ready',
              data: {
                ...data,
                replies:
                  unchanged && previous.state === 'ready' ? previous.data.replies : data.replies,
              },
            }));
            cursor = data.cursor;
          } else {
            // Keep the very first boundary; later coalesced reads must not
            // consume a batch that has not actually been shown.
            cursor ??= data.cursor;
          }
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
        iii.on<ReplyDelivery | { output: ReplyDelivery }>(base, (delivery) => {
          // Custom deliveries carry captured output; ADE may also project it.
          const event = delivery && 'output' in delivery ? delivery.output : delivery;
          if (!active || event?.claude_session_id !== id) return;
          // Deliveries are wakeups, not authoritative text: the active
          // branch may already have changed on the terminal host.
          void read();
        }),
      );
      cleanup.push(iii.on(`${base}-config`, unavailable));
      // The worker closes its gate when refresh starts, not when its reload
      // handler returns. Observe the same event, without its config values.
      const configChange = {
        function_id: 'configuration::on-updated',
        filters: { 'input.id': { $eq: configId } },
        capture: [],
      };
      cleanup.push(iii.observe(configChange, `${base}-config::${iii.browserId}`));
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
      // The cursor-based heartbeat also catches replies written before the
      // asynchronous observer baseline completed, or missed by a delivery.
      const timer = window.setInterval(() => void read(), 30000);
      cleanup.push(() => window.clearInterval(timer));
      void read();
    } catch {
      unavailable();
    }
    return stop;
  }, [iii, id, configId, attempt]);

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
  const replies = data.replies.length ? data.replies : data.last_reply ? [data.last_reply] : [];
  return (
    <div className="claude-follow-content">
      <p className="claude-follow-state">
        {data.name ?? id.slice(0, 8)} · {(STATUS[data.status] ?? STATUS.unknown).word}
      </p>
      <h3>{replies.length > 1 ? 'Assistant replies' : 'Latest assistant reply'}</h3>
      <div role="log" aria-label="Assistant replies">
        {replies.length ? (
          replies.map((reply) => (
            <p key={reply.id} className="claude-follow-text">
              {reply.text}
            </p>
          ))
        ) : (
          <p className="claude-follow-text">No assistant reply yet.</p>
        )}
      </div>
    </div>
  );
}

/** A modal keeps keystrokes out of the unrelated terminal behind the viewer. */
export function SessionDetail({
  iii,
  id,
  configId,
  available,
  opener,
  onClose,
}: {
  iii: ExtensionIii;
  id: string;
  configId?: string;
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
      <DialogContent
        showCloseButton={false}
        initialFocus={close}
        finalFocus={opener}
        onKeyDown={(event) => {
          // The portalled dialog also bubbles through the narrow Sessions drawer.
          if (event.key === 'Escape') event.stopPropagation();
        }}
      >
        <DialogTitle>Follow original</DialogTitle>
        <DialogDescription>
          Read only. ADE can’t verify that terminal input and output stay with this conversation.
          Use its original terminal to continue.
        </DialogDescription>
        {available && configId ? (
          <Follow key={configId} iii={iii} id={id} configId={configId} />
        ) : (
          <StatusPanel
            role="alert"
            variant="warn"
            headline="Replies unavailable"
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
