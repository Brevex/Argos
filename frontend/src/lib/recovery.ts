import { batch, createSignal, onCleanup } from 'solid-js';
import type { UnlistenFn } from '@tauri-apps/api/event';
import {
  type ProgressEvent,
  type SessionCompletionStatus,
  cancelRecovery,
  friendlyError,
  onArtifact,
  onProgress,
  onSessionCompleted,
  startRecovery,
} from './bridge';

export type SessionPhase =
  | 'idle'
  | 'starting'
  | 'running'
  | 'cancelling'
  | 'completed'
  | 'failed';

export interface RecoverySession {
  phase: () => SessionPhase;
  progress: () => ProgressEvent | null;
  bytesRecovered: () => number;
  elapsedMs: () => number;
  errorMessage: () => string | null;
  start: (source: string, output: string) => Promise<void>;
  cancel: () => Promise<void>;
  reset: () => void;
}

const COMPLETED_PHASE: Record<SessionCompletionStatus, SessionPhase> = {
  ok: 'completed',
  cancelled: 'completed',
  failed: 'failed',
};

const NON_FAILURE_MESSAGE: Record<'ok' | 'cancelled', string | null> = {
  ok: null,
  cancelled: 'Sessão cancelada antes da conclusão.',
};

export function createRecoverySession(): RecoverySession {
  const [phase, setPhase] = createSignal<SessionPhase>('idle');
  const [progress, setProgress] = createSignal<ProgressEvent | null>(null);
  const [bytesRecovered, setBytesRecovered] = createSignal(0);
  const [elapsedMs, setElapsedMs] = createSignal(0);
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [sessionId, setSessionId] = createSignal<number | null>(null);

  let unlistenProgress: UnlistenFn | undefined;
  let unlistenArtifact: UnlistenFn | undefined;
  let unlistenCompleted: UnlistenFn | undefined;
  let startedAt = 0;
  let tickHandle: number | undefined;

  const stopTick = () => {
    if (tickHandle !== undefined) {
      clearInterval(tickHandle);
      tickHandle = undefined;
    }
  };

  const startTick = () => {
    stopTick();
    startedAt = performance.now();
    setElapsedMs(0);
    tickHandle = window.setInterval(() => {
      setElapsedMs(performance.now() - startedAt);
    }, 500);
  };

  const detach = async () => {
    await Promise.all([
      unlistenProgress?.(),
      unlistenArtifact?.(),
      unlistenCompleted?.(),
    ]);
    unlistenProgress = undefined;
    unlistenArtifact = undefined;
    unlistenCompleted = undefined;
  };

  const reset = () => {
    stopTick();
    void detach();
    batch(() => {
      setPhase('idle');
      setProgress(null);
      setBytesRecovered(0);
      setElapsedMs(0);
      setErrorMessage(null);
      setSessionId(null);
    });
  };

  const start = async (source: string, output: string) => {
    if (phase() === 'starting' || phase() === 'running') return;
    await detach();
    batch(() => {
      setPhase('starting');
      setProgress(null);
      setBytesRecovered(0);
      setElapsedMs(0);
      setErrorMessage(null);
    });

    try {
      unlistenProgress = await onProgress((event) => setProgress(event));
      unlistenArtifact = await onArtifact((event) => {
        setBytesRecovered((b) => b + event.length);
      });
      unlistenCompleted = await onSessionCompleted((event) => {
        if (sessionId() !== event.session_id) return;
        stopTick();
        batch(() => {
          setPhase(COMPLETED_PHASE[event.status]);
          setErrorMessage(
            event.status === 'failed'
              ? friendlyError(event.error)
              : NON_FAILURE_MESSAGE[event.status],
          );
        });
      });

      const response = await startRecovery(source, output);
      batch(() => {
        setSessionId(response.session_id);
        setPhase('running');
      });
      startTick();
    } catch (e) {
      stopTick();
      await detach();
      batch(() => {
        setPhase('failed');
        setErrorMessage(friendlyError(e));
      });
    }
  };

  const cancel = async () => {
    const id = sessionId();
    if (id === null || phase() !== 'running') return;
    setPhase('cancelling');
    try {
      await cancelRecovery(id);
    } catch (e) {
      stopTick();
      batch(() => {
        setPhase('failed');
        setErrorMessage(friendlyError(e));
      });
    }
  };

  onCleanup(() => {
    stopTick();
    void detach();
  });

  return {
    phase,
    progress,
    bytesRecovered,
    elapsedMs,
    errorMessage,
    start,
    cancel,
    reset,
  };
}
