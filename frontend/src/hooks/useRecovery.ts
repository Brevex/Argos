import { createSignal, onCleanup } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface ProgressEvent {
  session_id: number;
  bytes_scanned: number;
  candidates_found: number;
  artifacts_recovered: number;
}

export interface ArtifactEvent {
  session_id: number;
  offset: number;
  length: number;
  format: string;
  score: number;
}

export function useRecovery() {
  const [isRunning, setIsRunning] = createSignal(false);
  const [progress, setProgress] = createSignal<ProgressEvent | null>(null);
  const [artifacts, setArtifacts] = createSignal<ArtifactEvent[]>([]);
  const [sessionId, setSessionId] = createSignal<number | null>(null);

  let unlistenProgress: UnlistenFn | undefined;
  let unlistenArtifact: UnlistenFn | undefined;

  const start = async (source: string, output: string) => {
    setArtifacts([]);
    setProgress(null);
    setIsRunning(true);

    unlistenProgress = await listen<ProgressEvent>('progress', (event) => {
      setProgress(event.payload);
    });

    unlistenArtifact = await listen<ArtifactEvent>('artifact', (event) => {
      setArtifacts((prev) => [...prev, event.payload]);
    });

    const response = await invoke<{ session_id: number }>('start_recovery', {
      request: { source, output },
    });
    setSessionId(response.session_id);
  };

  const cancel = async () => {
    const id = sessionId();
    if (id !== null) {
      await invoke('cancel_recovery', {
        request: { session_id: id },
      });
    }
  };

  onCleanup(() => {
    unlistenProgress?.();
    unlistenArtifact?.();
  });

  return { isRunning, progress, artifacts, start, cancel };
}
