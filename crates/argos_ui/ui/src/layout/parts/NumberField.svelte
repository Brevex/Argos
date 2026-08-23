<script lang="ts">
  /**
   * A number field with the stepper the theme draws, rather than the one the
   * platform draws.
   *
   * The control is still `input[type=number]`: it validates, it takes the
   * keyboard's arrows, and a screen reader announces it as a spin button. What
   * changes is who paints the two arrows — left to the platform they are the
   * one part of the settings panel wearing another system's clothes, and they
   * cannot be reached by any theme.
   *
   * An empty box is not zero. It means "take the engine's default", which is
   * why the value is `number | null` and why clearing the field is a thing a
   * person is allowed to do.
   */
  import { active } from '../../themes/active.svelte';

  let {
    value,
    placeholder,
    min = 0,
    disabled = false,
    label,
    onChange,
  }: {
    value: number | null;
    placeholder: string;
    min?: number;
    disabled?: boolean;
    /** Named for the assistive tree, since the words around it are not a label. */
    label: string;
    onChange: (value: number | null) => void;
  } = $props();

  let field: HTMLInputElement | undefined = $state();

  /** Reads the box, treating an empty one as "take the default". */
  function entered(text: string): number | null {
    if (text.trim() === '') return null;
    const parsed = Number(text);
    return Number.isFinite(parsed) ? parsed : null;
  }

  /**
   * One step up or down.
   *
   * From the placeholder when the box is empty, because a stepper that starts
   * at nothing and produces one is a stepper that lost the default it was
   * showing.
   */
  function nudge(by: number): void {
    const from = value ?? Number(placeholder);
    const next = Math.max(min, (Number.isFinite(from) ? from : min) + by);
    onChange(next);
    if (field !== undefined) field.value = String(next);
  }
</script>

<span class="spin" class:disabled>
  <input
    bind:this={field}
    class="number"
    type="number"
    {min}
    step="1"
    {placeholder}
    {disabled}
    aria-label={label}
    value={value ?? ''}
    onchange={(event) => onChange(entered(event.currentTarget.value))}
  />
  <span class="steps">
    <button type="button" {disabled} aria-label="{label}, one more" onclick={() => nudge(1)}>
      <svg viewBox="0 0 8 6" aria-hidden="true">{@html active.icon('up')}</svg>
    </button>
    <button type="button" {disabled} aria-label="{label}, one less" onclick={() => nudge(-1)}>
      <svg viewBox="0 0 8 6" aria-hidden="true">{@html active.icon('down')}</svg>
    </button>
  </span>
</span>

<style>
  .spin {
    display: inline-flex;
    align-items: stretch;
    width: 4.6rem;
    height: 1.5rem;
    background: var(--inset);
    border: 1px solid var(--inset-border);
    border-radius: var(--input-radius);
    box-shadow: var(--inset-shadow);
    overflow: hidden;
  }

  .spin:hover:not(.disabled) {
    border-color: var(--inset-border-hover);
  }

  .spin:focus-within {
    border-color: var(--inset-border-hover);
  }

  .spin.disabled {
    opacity: var(--disabled-opacity);
  }

  .number {
    flex: 1;
    min-width: 0;
    padding: 0 0.4rem;
    background: none;
    border: 0;
    color: var(--text);
    font: inherit;
    font-size: var(--type-sm);
    /* The platform's own arrows, gone: the two below replace them. */
    appearance: textfield;
    -moz-appearance: textfield;
  }

  .number::-webkit-outer-spin-button,
  .number::-webkit-inner-spin-button {
    appearance: none;
    -webkit-appearance: none;
    margin: 0;
  }

  .number:focus {
    outline: none;
  }

  .number:disabled {
    color: var(--disabled-text);
  }

  /* Two half-height buttons in the width of one, on the edge of the field —
     which is where this control has kept them for thirty years. */
  .steps {
    display: flex;
    flex: none;
    flex-direction: column;
    width: 1.05rem;
    border-left: 1px solid var(--inset-border);
  }

  .steps button {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 0;
    background: var(--button);
    border: 0;
    color: var(--button-text);
    cursor: pointer;
  }

  .steps button + button {
    border-top: 1px solid var(--inset-border);
  }

  .steps button:hover:not(:disabled) {
    background: var(--button-hover);
    color: var(--button-text-hover);
  }

  .steps button:active:not(:disabled) {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
    color: var(--button-text-hover);
  }

  .steps button:disabled {
    color: var(--disabled-text);
    cursor: default;
  }

  .steps button:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -2px;
  }

  .steps svg {
    width: 0.5rem;
    height: 0.38rem;
    fill: currentColor;
  }
</style>
