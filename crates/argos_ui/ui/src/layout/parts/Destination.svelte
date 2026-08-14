<script lang="ts">
  /**
   * Block two: where the job's output is written.
   *
   * A recovery writes images into a folder; an acquisition writes one image
   * file. Both are the same question — where does this go — so they are the
   * same block rather than two, and only the picker and the words change.
   *
   * Required, and required for a reason the engine enforces rather than this
   * field: a destination inside the medium under analysis would write onto the
   * evidence, and the engine refuses that before it creates anything. Browsing
   * here grants the window no filesystem access — the picker returns a path,
   * and the path is a string like any other.
   */
  import { open, save } from '@tauri-apps/plugin-dialog';

  import * as ipc from '../../lib/ipc';

  let {
    value,
    disabled,
    kind = 'folder',
    onChange,
  }: {
    value: string;
    disabled: boolean;
    /** `folder` for recovered images, `image` for a disk copy. */
    kind?: 'folder' | 'image';
    onChange: (path: string) => void;
  } = $props();

  const words = $derived(
    kind === 'image'
      ? {
          title: 'Image file',
          hint: 'Choose where to write the copy — not on the drive being copied',
          picker: 'Write the disk copy to',
        }
      : {
          title: 'Destination folder',
          hint: 'Choose a folder outside the drive being scanned',
          picker: 'Destination folder',
        },
  );

  async function browse(): Promise<void> {
    // The window runs as an administrator, so a picker left to its own devices
    // opens in the administrator's home rather than in the home of the person
    // looking at it.
    const start = value !== '' ? value : await ipc.invokerHome().catch(() => '');
    const defaultPath = start === '' ? undefined : start;
    // An acquisition names a file that must not exist yet, which is a save
    // dialog; a recovery names a folder that must, which is an open dialog.
    const picked =
      kind === 'image'
        ? await save({ title: words.picker, defaultPath })
        : await open({ directory: true, multiple: false, title: words.picker, defaultPath });
    if (typeof picked === 'string') onChange(picked);
  }
</script>

<section>
  <h2>{words.title} <span class="required">(required)</span></h2>

  <div class="field" class:disabled>
    <input
      value={value}
      {disabled}
      placeholder={words.hint}
      spellcheck="false"
      oninput={(event) => onChange(event.currentTarget.value)}
      aria-label={words.title}
    />
    <button {disabled} onclick={browse}>Browse</button>
  </div>
</section>

<style>
  h2 {
    font-size: 0.88rem;
    text-shadow: var(--text-glow);
    font-weight: 400;
    color: var(--text-dim);
    margin: 0 0 0.625rem;
  }

  .required {
    color: var(--text-faint);
  }

  .field {
    display: flex;
    align-items: stretch;
    border: 1px solid var(--inset-border);
    border-radius: var(--radius);
    background: var(--scanlines), var(--inset);
    overflow: hidden;
  }

  .field:focus-within {
    border-color: var(--row-selected-border);
  }

  .field.disabled {
    opacity: 0.78;
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 0.75rem 1rem;
    background: none;
    border: 0;
    color: var(--text);
    font-family: var(--font);
    font-size: 0.84rem;
  }

  input::placeholder {
    color: var(--text-faint);
  }

  input:focus {
    outline: none;
  }

  button {
    flex: none;
    padding: 0 1.375rem;
    background: var(--row-hover);
    border: 0;
    border-left: 1px solid var(--inset-border);
    color: var(--text);
    font: inherit;
    font-size: 0.84rem;
    cursor: pointer;
  }

  button:hover:not(:disabled) {
    background: var(--row-selected);
  }

  button:disabled {
    cursor: not-allowed;
  }
</style>
