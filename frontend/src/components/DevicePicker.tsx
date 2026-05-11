import { For, Show, createEffect, createResource } from 'solid-js';
import { type DeviceInfo, friendlyError, listDevices } from '../lib/bridge';
import { formatBytes } from '../lib/format';
import { ChevronIcon, DriveIcon } from './icons';

interface DevicePickerProps {
  selected: DeviceInfo | null;
  disabled: boolean;
  onSelect: (device: DeviceInfo) => void;
  onError: (message: string) => void;
}

const classLabel = (cls: DeviceInfo['class']): string => {
  if (cls === 'ssd') return 'SSD';
  if (cls === 'hdd') return 'HDD';
  return 'UNKNOWN';
};

export default function DevicePicker(props: DevicePickerProps) {
  const [devices, { refetch }] = createResource<DeviceInfo[]>(listDevices);

  createEffect(() => {
    const err = devices.error;
    if (err) props.onError(friendlyError(err));
  });

  return (
    <>
      <header class="section-title">
        <h2>Storage Devices</h2>
        <button
          class="btn ghost"
          type="button"
          onClick={() => void refetch()}
          disabled={devices.loading}
        >
          {devices.loading ? 'Refreshing…' : 'Refresh'}
        </button>
      </header>

      <div class="device-list">
        <Show
          when={!devices.loading && (devices() ?? []).length > 0}
          fallback={
            <div class="empty-state">
              <Show when={devices.loading} fallback="Connect a device to see it listed.">
                Reading devices…
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
                  <DriveIcon />
                </span>
                <span class="device-meta">
                  <span class="device-name">
                    {device.path}
                    <Show when={device.removable}>
                      <span class="tag removable">Removable</span>
                    </Show>
                  </span>
                  <span class="device-model">
                    <span class={`tag ${device.class}`}>
                      {classLabel(device.class)}
                    </span>
                    <span class="device-size">
                      {device.model ?? device.name} · {formatBytes(device.size_bytes)}
                    </span>
                  </span>
                </span>
                <span class="device-chevron" aria-hidden="true">
                  <ChevronIcon />
                </span>
              </button>
            )}
          </For>
        </Show>
      </div>
    </>
  );
}
