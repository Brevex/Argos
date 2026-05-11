import { createEffect, createMemo, createSignal } from 'solid-js';
import Background from './components/Background';
import Glass from './components/Glass';
import DevicePicker from './components/DevicePicker';
import OutputPicker from './components/OutputPicker';
import StatusPanel from './components/StatusPanel';
import IntegrityFooter from './components/IntegrityFooter';
import ErrorModal from './components/ErrorModal';
import type { DeviceInfo } from './lib/bridge';
import { createRecoverySession } from './lib/recovery';

export default function App() {
  const session = createRecoverySession();
  const [device, setDevice] = createSignal<DeviceInfo | null>(null);
  const [output, setOutput] = createSignal<string>('');
  const [modalError, setModalError] = createSignal<string | null>(null);

  createEffect(() => {
    const message = session.errorMessage();
    if (message) setModalError(message);
  });

  const isBusy = createMemo(
    () =>
      session.phase() === 'running' ||
      session.phase() === 'starting' ||
      session.phase() === 'cancelling',
  );

  const canStart = createMemo(
    () =>
      (session.phase() === 'idle' || session.phase() === 'completed') &&
      device() !== null &&
      output().length > 0,
  );

  const handleStart = () => {
    const d = device();
    const o = output();
    if (!d || !o) return;
    void session.start(d.path, o);
  };

  return (
    <div class="app-shell">
      <Background />
      <div class="workspace">
        <header class="workspace-hero">
          <h1>Image Recovery</h1>
          <p>
            Select a disk to start scanning for permanently deleted images.
          </p>
        </header>

        <div class="workspace-grid">
          <aside class="column-left">
            <Glass class="panel-output">
              <OutputPicker
                value={output()}
                disabled={isBusy()}
                onChange={setOutput}
                onError={setModalError}
              />
            </Glass>
            <Glass class="panel-devices">
              <DevicePicker
                selected={device()}
                disabled={isBusy()}
                onSelect={setDevice}
                onError={setModalError}
              />
            </Glass>
          </aside>

          <main class="column-right">
            <Glass class="panel-status">
              <StatusPanel
                phase={session.phase()}
                progress={session.progress()}
                device={device()}
                bytesRecovered={session.bytesRecovered()}
                elapsedMs={session.elapsedMs()}
                canStart={canStart()}
                onStart={handleStart}
                onCancel={() => void session.cancel()}
                onReset={session.reset}
              />
            </Glass>
          </main>
        </div>

        <IntegrityFooter phase={session.phase()} />
      </div>

      <ErrorModal
        message={modalError()}
        onClose={() => setModalError(null)}
      />
    </div>
  );
}
