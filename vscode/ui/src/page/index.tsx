import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Button,
  EmptyState,
  type Host,
  IconButton,
  List,
  ListItem,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  StatusPanel,
} from '@iii-dev/console-ui';
import { ExternalLink, Folder, FolderOpen, RefreshCw, Square, SquareCode } from './icons';

type Props = PageRenderProps & { host: Host };

export type Instance = {
  id: string;
  name: string;
  workspace: string;
  host: string;
  port: number;
  status: 'starting' | 'running' | 'stopped' | 'failed';
};

type Phase = 'starting' | 'ready' | 'stopped' | 'error';

type View = 'pick' | 'starting' | 'ready' | 'stopped' | 'error';

type PanelContext = { workspace?: string } | null | undefined;

const startTimeoutMs = 240_000;

export function workbenchUrl(instance: Pick<Instance, 'host' | 'port'>) {
  const host = instance.host.includes(':') ? `[${instance.host}]` : instance.host;
  return `http://${host}:${instance.port}/`;
}

function contextWorkspace(context: PanelContext) {
  const workspace = context?.workspace;
  return typeof workspace === 'string' && workspace.length > 0 ? workspace : null;
}

function resolveView(workspace: string | null, phase: Phase, instance: Instance | null): View {
  if (!workspace) return 'pick';
  if (phase === 'ready') return instance ? 'ready' : 'starting';
  return phase;
}

function describe(cause: unknown) {
  return cause instanceof Error ? cause.message : String(cause);
}

export function VscodePage({ host, onRequestClose, workingDir, panelContext, commands }: Props) {
  const [chosen, setChosen] = useState<string | null>(null);
  const [seenWorkingDir, setSeenWorkingDir] = useState(workingDir);
  const [instance, setInstance] = useState<Instance | null>(null);
  const [phase, setPhase] = useState<Phase>('starting');
  const [error, setError] = useState<string | null>(null);
  const requestSeq = useRef(0);
  const autoStarted = useRef<string | null>(null);

  if (workingDir !== seenWorkingDir) {
    setSeenWorkingDir(workingDir);
    setChosen(null);
  }

  const workspace = chosen ?? workingDir ?? null;
  const view = resolveView(workspace, phase, instance);
  const running = view === 'ready';
  const recent = useMemo(
    () =>
      (host.workspace?.recentDirectories() ?? []).filter((dir) => dir !== workspace).slice(0, 8),
    [host, workspace],
  );

  const start = useCallback(
    async (path: string) => {
      const seq = ++requestSeq.current;
      setPhase('starting');
      setError(null);
      try {
        const result = await host.iii.trigger<Instance>(
          'vscode::start',
          { workspace: path },
          { timeoutMs: startTimeoutMs },
        );
        if (seq !== requestSeq.current) return;
        setInstance(result);
        setPhase('ready');
      } catch (cause) {
        if (seq !== requestSeq.current) return;
        setError(describe(cause));
        setPhase('error');
      }
    },
    [host],
  );

  const stop = useCallback(async () => {
    if (!instance) return;
    const seq = ++requestSeq.current;
    try {
      await host.iii.trigger('vscode::stop', { id: instance.id });
      if (seq === requestSeq.current) setPhase('stopped');
    } catch (cause) {
      if (seq !== requestSeq.current) return;
      setError(describe(cause));
      setPhase('error');
    }
  }, [host, instance]);

  const startCurrent = useCallback(() => {
    if (workspace) void start(workspace);
  }, [workspace, start]);

  const openExternal = useCallback(() => {
    if (instance) window.open(workbenchUrl(instance), '_blank', 'noopener');
  }, [instance]);

  useEffect(() => {
    const fromContext = contextWorkspace(panelContext?.context as PanelContext);
    if (fromContext) setChosen(fromContext);
  }, [panelContext?.id, panelContext?.context]);

  useEffect(() => {
    if (!workspace || autoStarted.current === workspace) return;
    autoStarted.current = workspace;
    void start(workspace);
  }, [workspace, start]);

  useEffect(
    () =>
      commands?.register([
        {
          id: 'reload',
          title: 'Reload workbench',
          shortcut: 'R',
          enabled: () => running,
          run: startCurrent,
        },
        {
          id: 'open-external',
          title: 'Open in a browser tab',
          shortcut: 'O',
          enabled: () => running,
          run: openExternal,
        },
        {
          id: 'stop',
          title: 'Stop VS Code Server',
          shortcut: 'X',
          enabled: () => running,
          run: () => void stop(),
        },
        {
          id: 'start',
          title: 'Start VS Code',
          keywords: ['restart', 'retry'],
          enabled: () => view === 'stopped' || view === 'error',
          run: startCurrent,
        },
      ]),
    [commands, running, view, startCurrent, openExternal, stop],
  );

  const actions = [
    { label: 'Reload workbench', onClick: startCurrent, Icon: RefreshCw },
    { label: 'Open in a browser tab', onClick: openExternal, Icon: ExternalLink },
    { label: 'Stop VS Code Server', onClick: () => void stop(), Icon: Square },
  ];

  const state = (() => {
    switch (view) {
      case 'pick':
        return (
          <>
            <EmptyState
              icon={FolderOpen}
              title="Pick a folder to open"
              description="VS Code follows the chat's working directory. Set one in chat, or open a recent folder here."
            />
            {recent.length > 0 && (
              <List className="vscode-folders" aria-label="Recent folders">
                {recent.map((dir, index) => (
                  <ListItem
                    key={dir}
                    leading={<Folder />}
                    label={<span className="vscode-path">{dir}</span>}
                    data-autofocus={index === 0 ? '' : undefined}
                    onClick={() => setChosen(dir)}
                  />
                ))}
              </List>
            )}
          </>
        );
      case 'starting':
        return (
          <StatusPanel
            variant="info"
            headline="Starting VS Code"
            detail="The first launch downloads the matching VS Code Server, which can take a minute."
          />
        );
      case 'error':
        return (
          <>
            <StatusPanel variant="alert" headline="VS Code did not start" detail={error} />
            <Button variant="primary" data-autofocus="" onClick={startCurrent}>
              Try again
            </Button>
          </>
        );
      case 'stopped':
        return (
          <EmptyState
            icon={SquareCode}
            title="VS Code Server stopped"
            description="The process for this folder was stopped. Start it again to reopen the workbench."
            action={{ label: 'Start VS Code', onClick: startCurrent }}
          />
        );
      default:
        return null;
    }
  })();

  return (
    <PageShell>
      <PageHeader
        icon={<SquareCode />}
        title="VS Code"
        description={
          workspace ? (
            <span className="vscode-path">{workspace}</span>
          ) : (
            'VS Code for the active working directory'
          )
        }
        actions={actions.map(({ label, onClick, Icon }) => (
          <IconButton
            key={label}
            label={label}
            variant="ghost"
            onClick={onClick}
            disabled={!running}
          >
            <Icon />
          </IconButton>
        ))}
        onClose={onRequestClose}
      />
      <PageMain className="vscode-main">
        {state && <div className="vscode-state">{state}</div>}
        {running && instance && (
          <iframe
            className="vscode-frame"
            src={workbenchUrl(instance)}
            title={`VS Code ${instance.workspace}`}
            allow="clipboard-read; clipboard-write"
            data-autofocus=""
          />
        )}
      </PageMain>
    </PageShell>
  );
}
