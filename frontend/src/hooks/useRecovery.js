import { createSignal, onCleanup } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export function useRecovery() {
    const [isRunning, setIsRunning] = createSignal(false);
    const [progress, setProgress] = createSignal(null);
    const [artifacts, setArtifacts] = createSignal([]);
    const [sessionId, setSessionId] = createSignal(null);
    let unlistenProgress;
    let unlistenArtifact;
    const start = async (source, output) => {
        setArtifacts([]);
        setProgress(null);
        setIsRunning(true);
        unlistenProgress = await listen('progress', (event) => {
            setProgress(event.payload);
        });
        unlistenArtifact = await listen('artifact', (event) => {
            setArtifacts((prev) => [...prev, event.payload]);
        });
        const response = await invoke('start_recovery', {
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
