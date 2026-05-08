import { createSignal, For, Show } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import { useRecovery } from '../hooks/useRecovery';

export default function RecoveryPanel() {
  const { isRunning, progress, artifacts, start, cancel } = useRecovery();
  const [source, setSource] = createSignal('');
  const [output, setOutput] = createSignal('');

  const pickSource = async () => {
    const path = await open({ directory: false });
    if (typeof path === 'string') {
      setSource(path);
    }
  };

  const pickOutput = async () => {
    const path = await open({ directory: true });
    if (typeof path === 'string') {
      setOutput(path);
    }
  };

  return (
    <section>
      <div>
        <button onClick={pickSource} disabled={isRunning()}>
          Select Source
        </button>
        <span>{source()}</span>
      </div>
      <div>
        <button onClick={pickOutput} disabled={isRunning()}>
          Select Output
        </button>
        <span>{output()}</span>
      </div>
      <button
        onClick={() => start(source(), output())}
        disabled={isRunning() || !source() || !output()}
      >
        Start
      </button>
      <button onClick={cancel} disabled={!isRunning()}>
        Cancel
      </button>

      <Show when={progress()}>
        <div>
          Scanned: {progress()?.bytes_scanned} |
          Candidates: {progress()?.candidates_found} |
          Recovered: {progress()?.artifacts_recovered}
        </div>
      </Show>

      <ul>
        <For each={artifacts()}>
          {(artifact) => (
            <li>
              {artifact.format} @ {artifact.offset} (
              {artifact.length} bytes) — score:{' '}
              {artifact.score.toFixed(2)}
            </li>
          )}
        </For>
      </ul>
    </section>
  );
}
