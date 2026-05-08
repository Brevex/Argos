import { createMemo, createSignal } from 'solid-js';
import Background from './components/Background';
import Glass from './components/Glass';
import DevicePicker from './components/DevicePicker';
import OutputPicker from './components/OutputPicker';
import ProgressView from './components/ProgressView';
import BottomBanner from './components/BottomBanner';
import type { DeviceInfo } from './lib/bridge';
import { createRecoverySession } from './lib/recovery';

export default function App() {
  const session = createRecoverySession();
  const [device, setDevice] = createSignal<DeviceInfo | null>(null);
  const [output, setOutput] = createSignal<string>('');

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
        <div class="workspace-grid">
          <Glass class="panel-devices">
            <DevicePicker
              selected={device()}
              disabled={isBusy()}
              onSelect={setDevice}
            />
          </Glass>
          <Glass class="panel-progress">
            <ProgressView
              phase={session.phase()}
              progress={session.progress()}
              device={device()}
              bytesRecovered={session.bytesRecovered()}
              elapsedMs={session.elapsedMs()}
              errorMessage={session.errorMessage()}
            />
          </Glass>
          <Glass class="panel-output">
            <OutputPicker
              value={output()}
              disabled={isBusy()}
              onChange={setOutput}
            />
          </Glass>
        </div>
        <BottomBanner
          phase={session.phase()}
          canStart={canStart()}
          onStart={handleStart}
          onCancel={() => void session.cancel()}
          onReset={session.reset}
        />
      </div>
    </div>
  );
}
