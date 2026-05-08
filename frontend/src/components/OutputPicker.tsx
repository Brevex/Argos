import { Show } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';

interface OutputPickerProps {
  value: string;
  disabled: boolean;
  onChange: (path: string) => void;
}

function FolderIcon() {
  return (
    <svg viewBox="0 0 24 24" width="20" height="20" fill="none" aria-hidden="true">
      <path
        d="M3.5 7.5C3.5 6.39543 4.39543 5.5 5.5 5.5H10L12 7.5H18.5C19.6046 7.5 20.5 8.39543 20.5 9.5V16.5C20.5 17.6046 19.6046 18.5 18.5 18.5H5.5C4.39543 18.5 3.5 17.6046 3.5 16.5V7.5Z"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linejoin="round"
      />
    </svg>
  );
}

export default function OutputPicker(props: OutputPickerProps) {
  const pick = async () => {
    const result = await open({
      directory: true,
      multiple: false,
      title: 'Selecionar pasta de destino',
    });
    if (typeof result === 'string') {
      props.onChange(result);
    }
  };

  return (
    <>
      <header class="section-title">
        <h2>Destino dos arquivos recuperados</h2>
        <span class="hint">Sistema de arquivos distinto do dispositivo</span>
      </header>

      <div class="output-row">
        <span class="output-icon" aria-hidden="true">
          <FolderIcon />
        </span>
        <span class={`output-path ${props.value ? '' : 'empty'}`}>
          <Show when={props.value} fallback="Nenhuma pasta selecionada">
            {props.value}
          </Show>
        </span>
        <button
          type="button"
          class="btn"
          onClick={() => void pick()}
          disabled={props.disabled}
        >
          {props.value ? 'Trocar pasta' : 'Escolher pasta'}
        </button>
      </div>
    </>
  );
}
