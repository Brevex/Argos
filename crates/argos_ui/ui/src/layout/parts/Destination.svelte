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
    font-size: var(--type-md);
    text-shadow: var(--text-glow);
    font-weight: 400;
    color: var(--heading);
    margin: 0 0 0.625rem;
  }

  .required {
    color: var(--text-faint);
  }

  /* A field is cut into the sheet rather than laid on it: the theme's sunken
     bevel, and the button attached to its right edge shares the cut. */
  .field {
    display: flex;
    align-items: stretch;
    border: 1px solid var(--inset-border);
    border-radius: var(--input-radius);
    background: var(--scanlines), var(--inset);
    box-shadow: var(--inset-shadow);
    overflow: hidden;
  }

  .field:hover:not(.disabled) {
    border-color: var(--inset-border-hover);
  }

  .field:focus-within {
    border-color: var(--inset-border-hover);
  }

  .field.disabled {
    border-color: var(--inset-border);
    opacity: var(--disabled-opacity);
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 0.75rem 1rem;
    background: none;
    border: 0;
    color: var(--text);
    font-family: var(--font);
    font-size: var(--type-md);
  }

  input::placeholder {
    color: var(--text-faint);
  }

  input:disabled {
    color: var(--disabled-text);
  }

  /* The field's own button: the ordinary one, attached. It keeps the button
     fill so it reads as something to press, and gives up its corners on the
     side that meets the field. */
  button {
    position: relative;
    flex: none;
    padding: 0 1.375rem;
    background: var(--button);
    border: 0;
    border-left: 1px solid var(--inset-border);
    color: var(--button-text);
    font: inherit;
    font-size: var(--type-md);
    cursor: pointer;
  }

  button::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: var(--specular);
  }

  button:hover:not(:disabled) {
    background: var(--button-hover);
    color: var(--button-text-hover);
  }

  button:active:not(:disabled) {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
    color: var(--button-text-hover);
  }

  button:disabled {
    color: var(--disabled-text);
    cursor: not-allowed;
  }

  button:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -3px;
  }
</style>
