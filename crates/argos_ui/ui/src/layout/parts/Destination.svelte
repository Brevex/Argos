<script lang="ts">
  /**
   * Block two: where recovered images are written.
   *
   * Required, and required for a reason the engine enforces rather than this
   * field: a destination inside the medium under analysis would write onto the
   * evidence, and the engine refuses that before it creates anything. Browsing
   * to a folder here grants the window no filesystem access — the picker
   * returns a path, and the path is a string like any other.
   */
  import { open } from '@tauri-apps/plugin-dialog';

  let {
    value,
    disabled,
    onChange,
  }: { value: string; disabled: boolean; onChange: (path: string) => void } = $props();

  async function browse(): Promise<void> {
    const picked = await open({ directory: true, multiple: false, title: 'Destination folder' });
    if (typeof picked === 'string') onChange(picked);
  }
</script>

<section>
  <h2>Destination folder <span class="required">(required)</span></h2>

  <div class="field" class:disabled>
    <input
      value={value}
      {disabled}
      placeholder="Choose a folder outside the drive being scanned"
      spellcheck="false"
      oninput={(event) => onChange(event.currentTarget.value)}
      aria-label="Destination folder"
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
