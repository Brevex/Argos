import { Show, createResource } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import { defaultOutputDir } from '../lib/bridge';
import { FolderIcon } from './icons';

interface OutputPickerProps {
  value: string;
  disabled: boolean;
  onChange: (path: string) => void;
  onError: (message: string) => void;
}

export default function OutputPicker(props: OutputPickerProps) {
  const [home] = createResource(defaultOutputDir);

  const pick = async () => {
    try {
      const result = await open({
        directory: true,
        multiple: false,
        title: 'Select destination folder',
        defaultPath: props.value || home() || undefined,
      });
      if (typeof result === 'string') {
        props.onChange(result);
      }
    } catch {
      props.onError('Failed to open the system folder dialog.');
    }
  };

  return (
    <div class="output-picker">
      <span class="output-icon" aria-hidden="true">
        <FolderIcon />
      </span>
      <div class="output-meta">
        <span class="output-label">Output folder</span>
        <span class={`output-path ${props.value ? '' : 'empty'}`}>
          <Show when={props.value} fallback="No folder selected">
            {props.value}
          </Show>
        </span>
      </div>
      <button
        type="button"
        class="btn"
        onClick={() => void pick()}
        disabled={props.disabled}
      >
        {props.value ? 'Change' : 'Choose folder'}
      </button>
    </div>
  );
}
