<script lang="ts">
  /**
   * Block one: the media a scan can be pointed at.
   *
   * Every column is copied from what the engine reported. This table decides
   * nothing about a medium — not its class, not whether being mounted matters.
   * It repeats, because those are forensic judgements and they belong to the
   * engine (`A-SHELL-NO-DOMAIN`).
   */
  import type { Device } from '../../lib/dto';
  import { bytes } from '../../lib/format';

  let {
    devices,
    selected,
    disabled,
    refreshing,
    onSelect,
    onRefresh,
    onConfig,
  }: {
    devices: Device[];
    selected: string;
    disabled: boolean;
    refreshing: boolean;
    onSelect: (path: string) => void;
    onRefresh: () => void;
    onConfig: () => void;
  } = $props();
</script>

<section>
  <header>
    <h2>Select drive to scan</h2>
    <button
      class="refresh"
      disabled={disabled || refreshing}
      onclick={onRefresh}
      title="Read the list of disks again"
    >
      {refreshing ? 'Refreshing…' : 'Refresh'}
    </button>

    <button class="config" onclick={onConfig} title="Appearance">Config</button>
  </header>

  <div class="table" role="radiogroup" aria-label="Select drive to scan">
    {#if devices.length === 0}
      <p class="empty">
        No disks listed. Enumeration needs no privileges, so an empty list means
        this platform does not publish one — not that access was refused. A path
        can still be typed, and a raw image file always works.
      </p>
    {/if}

    {#each devices as device (device.path)}
      <button
        class="row"
        class:selected={selected === device.path}
        role="radio"
        aria-checked={selected === device.path}
        {disabled}
        onclick={() => onSelect(device.path)}
      >
        <span class="dot" aria-hidden="true"></span>

        <span class="name">{device.path}</span>
        <span class="kind">
          {device.class}
          <!--
            The engine reported this medium trims. That is a fact about the
            medium and it belongs beside the medium's class, which is the other
            fact about it — not in a message under the statistics, where it
            arrives after the drive has already been chosen. What it means for
            a recovery is the engine's to say, and the engine says it.
          -->
          {#if device.trim === 'enabled'}
            <span class="seal">TRIM</span>
          {/if}
        </span>
        <span class="size">
          {device.capacityBytes === null ? 'size unknown' : bytes(device.capacityBytes)}
        </span>

        <span class="mounted">
          {#if device.mounts.length > 0}
            <span class="seal" class:quiet={!device.writableMount}>
              {device.writableMount ? 'mounted, writable' : 'mounted read-only'}
            </span>
          {/if}
        </span>
      </button>
    {/each}
  </div>
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.625rem;
  }

  h2 {
    font-size: var(--type-md);
    font-weight: 400;
    color: var(--heading);
    margin: 0;
    text-shadow: var(--text-glow);
  }

  /* Opposite ends of one line: what re-reads the machine on the left, beside
     the heading it belongs to, and what changes the window's appearance at the
     far right, away from anything that touches a disk. */
  .config {
    margin-left: auto;
  }

  /* Both are the ordinary button — the same object as Browse and as everything
     in the results view. There is one of these in this interface, not four. */
  .refresh,
  .config {
    position: relative;
    /* Wide enough for the longer of the two labels, so pressing it does not
       shift the heading beside it. */
    min-width: 5.5rem;
    padding: 0.25rem 0.75rem;
    background: var(--button);
    border: 1px solid var(--button-border);
    border-radius: var(--button-radius);
    box-shadow: var(--button-shadow);
    color: var(--button-text);
    font-family: var(--font);
    font-size: var(--type-xs);
    cursor: pointer;
  }

  .refresh::after,
  .config::after {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    border-radius: inherit;
    background: var(--specular);
  }

  .refresh:hover:not(:disabled),
  .config:hover:not(:disabled) {
    background: var(--button-hover);
    border-color: var(--button-border-hover);
    color: var(--button-text-hover);
  }

  .refresh:active:not(:disabled),
  .config:active:not(:disabled) {
    background: var(--button-active);
    box-shadow: var(--button-shadow-active);
    color: var(--button-text-hover);
  }

  .refresh:disabled {
    cursor: not-allowed;
    color: var(--disabled-text);
    opacity: var(--disabled-opacity);
  }

  .table {
    /* Rows have a set height and the list stops after four of them, so this
       block is a known height on every window and a scrollbar appears only when
       a machine has a fifth disk. The row height scales with the window, so the
       block shrinks with everything else rather than forcing the window open.
       The quarter-rem of slack absorbs the rounding a fractional row height
       leaves behind, which is what would otherwise show a scrollbar for a list
       that fits. */
    max-height: calc(var(--row-height) * 4 + 0.25rem);
    border: 1px solid var(--inset-border);
    border-radius: var(--input-radius);
    background: var(--scanlines), var(--inset);
    box-shadow: var(--inset-shadow);
    overflow-y: auto;
  }

  .empty {
    margin: 0;
    padding: 1rem 1.125rem;
    font-size: var(--type-sm);
    line-height: 1.5;
    color: var(--text-faint);
    max-width: var(--measure);
  }

  .row {
    display: grid;
    grid-template-columns: 1rem minmax(0, 1.35fr) minmax(0, 1fr) minmax(0, 1fr) 8.6rem;
    align-items: center;
    gap: 0.875rem;
    width: 100%;
    height: var(--row-height);
    padding: 0 1rem;
    background: var(--row);
    border: 1px solid transparent;
    border-radius: var(--row-radius);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .row:hover:not(:disabled):not(.selected) {
    background: var(--row-hover);
  }

  .row:active:not(:disabled):not(.selected) {
    background: var(--row-selected);
  }

  .row:disabled {
    cursor: not-allowed;
    color: var(--disabled-text);
  }

  .row.selected {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
    box-shadow: var(--row-selected-shadow);
  }

  .row:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -3px;
  }

  /*
   * The mark on the drive that will be read, in whichever of the two shapes
   * the theme draws one.
   *
   * `dot` is a ring that fills; `radio` is the bevelled well of a desktop that
   * cut its controls into the surface; `bracket` is the caret a character
   * display put beside the current line, because it had nothing else to put
   * there. Same control, same place, same size.
   */
  .dot {
    width: 0.81rem;
    height: 0.81rem;
    justify-self: center;
    border-radius: 50%;
    border: 1.4px solid var(--choice-border);
    background: var(--choice);
    box-shadow: var(--choice-shadow);
  }

  .row.selected .dot {
    border-color: var(--choice-border-selected);
    background: var(--choice-mark);
  }

  :global(html[data-choice='dot']) .dot {
    background: transparent;
  }

  :global(html[data-choice='dot']) .row.selected .dot {
    background: var(--choice-mark);
  }

  /* A well with a bead in it. The whole drawing — the bead, its rim and the
     well behind it — is the theme's, because a bevelled desktop's radio is a
     different object from a flat one's dot. */
  :global(html[data-choice='radio']) .row.selected .dot {
    background: var(--choice-mark);
  }

  :global(html[data-choice='bracket']) .dot {
    width: auto;
    height: auto;
    border: 0;
    border-radius: 0;
    background: none;
    box-shadow: none;
    justify-self: start;
    font-family: var(--font);
    font-size: var(--type-md);
    line-height: 1;
    color: transparent;
  }

  :global(html[data-choice='bracket']) .dot::before {
    content: '>';
    white-space: pre;
  }

  :global(html[data-choice='bracket']) .row:hover .dot {
    color: var(--text-faint);
  }

  :global(html[data-choice='bracket']) .row.selected .dot {
    color: var(--accent-strong);
    text-shadow: var(--text-glow);
  }

  .name {
    font-size: var(--type-md);
    text-shadow: var(--text-glow);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind,
  .size {
    font-size: var(--type-md);
    color: var(--text-dim);
  }

  .kind {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
  }

  .mounted {
    display: flex;
    justify-content: flex-end;
    min-width: 0;
  }

  /*
   * A seal: one fact the operating system reported about this medium, said in
   * the fewest words it can be said in and sitting beside the other facts
   * about it.
   *
   * Two weights. The loud one is for what bears on whether a recovery can be
   * trusted — a medium that trims, a medium the system is writing to while it
   * is read. The quiet one describes the medium and asks nothing.
   */
  .seal {
    flex: none;
    padding: 0.05rem 0.36rem;
    font-size: var(--type-2xs);
    line-height: 1.5;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--badge-text);
    background: var(--badge);
    border: 1px solid var(--badge-border);
    border-radius: var(--badge-radius);
    text-shadow: var(--text-glow);
  }

  .seal.quiet {
    color: var(--badge-quiet-text);
    background: var(--badge-quiet);
    border-color: var(--badge-quiet-border);
  }
</style>
