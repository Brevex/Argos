import { For, Show, createResource } from 'solid-js';
import { type DeviceInfo, friendlyError, listDevices } from '../lib/bridge';
import { formatBytes } from '../lib/format';

interface DevicePickerProps {
  selected: DeviceInfo | null;
  disabled: boolean;
  onSelect: (device: DeviceInfo) => void;
}

const classLabel = (cls: DeviceInfo['class']): string => {
  if (cls === 'ssd') return 'SSD';
  if (cls === 'hdd') return 'HDD';
  return 'DESCONHECIDO';
};

function DeviceIcon() {
  return (
    <svg viewBox="0 0 24 24" width="22" height="22" fill="none" aria-hidden="true">
      <rect
        x="3"
        y="5"
        width="18"
        height="14"
        rx="2.5"
        stroke="currentColor"
        stroke-width="1.5"
      />
      <circle cx="17" cy="12" r="1.2" fill="currentColor" />
      <line
        x1="6"
        y1="9"
        x2="14"
        y2="9"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
      />
      <line
        x1="6"
        y1="12"
        x2="11"
        y2="12"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
      />
      <line
        x1="6"
        y1="15"
        x2="13"
        y2="15"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
      />
    </svg>
  );
}

export default function DevicePicker(props: DevicePickerProps) {
  const [devices, { refetch }] = createResource<DeviceInfo[]>(listDevices);

  return (
    <>
      <header class="section-title">
        <h2>Dispositivos de armazenamento</h2>
        <button
          class="btn ghost"
          type="button"
          onClick={() => void refetch()}
          disabled={devices.loading}
        >
          {devices.loading ? 'Atualizando…' : 'Atualizar'}
        </button>
      </header>

      <Show when={devices.error}>
        <div class="error-banner">
          <span class="error-banner-dot" />
          {friendlyError(devices.error)}
        </div>
      </Show>

      <div class="device-list">
        <Show
          when={!devices.loading && (devices() ?? []).length > 0}
          fallback={
            <div class="empty-state">
              <Show
                when={devices.loading}
                fallback="Nenhum dispositivo detectado."
              >
                Lendo dispositivos…
              </Show>
            </div>
          }
        >
          <For each={devices()}>
            {(device) => (
              <button
                type="button"
                class={`device-card ${
                  props.selected?.path === device.path ? 'selected' : ''
                }`}
                disabled={props.disabled}
                onClick={() => props.onSelect(device)}
              >
                <span class="device-icon" aria-hidden="true">
                  <DeviceIcon />
                </span>
                <span class="device-meta">
                  <span class="device-name">{device.path}</span>
                  <span class="device-model">
                    {device.model ?? device.name} · {formatBytes(device.size_bytes)}
                  </span>
                </span>
                <span class="device-tags">
                  <span class={`tag ${device.class}`}>
                    {classLabel(device.class)}
                  </span>
                  <Show when={device.removable}>
                    <span class="tag removable">Removível</span>
                  </Show>
                </span>
              </button>
            )}
          </For>
        </Show>
      </div>
    </>
  );
}
