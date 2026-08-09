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
  }: {
    devices: Device[];
    selected: string;
    disabled: boolean;
    refreshing: boolean;
    onSelect: (path: string) => void;
    onRefresh: () => void;
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

        <span class="disk" aria-hidden="true">
          <svg viewBox="0 0 20 16">
            <rect class="case" x="0.7" y="0.7" width="18.6" height="14.6" rx="2" />
            <circle class="platter" cx="8.2" cy="8" r="5" />
            <circle class="spindle" cx="8.2" cy="8" r="1.15" />
            <path class="arm" d="M16.4 4.1 12.1 9.7" />
            <circle class="pivot" cx="16.6" cy="3.6" r="1.05" />
          </svg>
        </span>

        <span class="name">{device.path}</span>
        <span class="kind">{device.class}</span>
        <span class="size">
          {device.capacityBytes === null ? 'size unknown' : bytes(device.capacityBytes)}
        </span>

        <span class="mounted" class:writable={device.writableMount}>
          {#if device.mounts.length > 0}
            {device.writableMount ? 'mounted, writable' : 'mounted read-only'}
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
    font-size: 0.81rem;
    font-weight: 400;
    color: var(--text-dim);
    margin: 0;
  }

  .refresh {
    /* Wide enough for the longer of the two labels, so pressing it does not
       shift the heading beside it. */
    min-width: 5.5rem;
    padding: 0.25rem 0.75rem;
    background: var(--row-hover);
    border: 1px solid var(--inset-border);
    border-radius: var(--radius);
    color: var(--text-dim);
    font-family: var(--font);
    font-size: 0.75rem;
    cursor: pointer;
  }

  .refresh:hover:not(:disabled) {
    background: var(--row-selected);
    color: var(--text);
  }

  .refresh:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .table {
    /* Rows have a set height and the list stops after three of them, so this
       block's height is the same on every window and a scrollbar appears only
       when a machine actually has a fourth disk. */
    --row-height: 2.65rem;
    max-height: calc(var(--row-height) * 3 + 2px);
    border: 1px solid var(--inset-border);
    border-radius: var(--radius);
    background: var(--inset);
    overflow-y: auto;
  }

  .empty {
    margin: 0;
    padding: 1rem 1.125rem;
    font-size: 0.78rem;
    color: var(--text-faint);
    max-width: 72ch;
  }

  .row {
    display: grid;
    grid-template-columns: 1rem 1.375rem minmax(0, 1.35fr) minmax(0, 1fr) minmax(0, 1fr) 8.6rem;
    align-items: center;
    gap: 0.875rem;
    width: 100%;
    height: var(--row-height);
    padding: 0 1rem;
    background: var(--row);
    border: 1px solid transparent;
    border-radius: var(--radius);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .row:hover:not(:disabled):not(.selected) {
    background: var(--row-hover);
  }

  .row:disabled {
    cursor: not-allowed;
  }

  .row.selected {
    background: var(--row-selected);
    border-color: var(--row-selected-border);
  }

  .dot {
    width: 0.81rem;
    height: 0.81rem;
    border-radius: 50%;
    border: 1.4px solid var(--text-faint);
    justify-self: center;
  }

  .row.selected .dot {
    border-color: var(--accent-strong);
    background:
      radial-gradient(circle at 50% 50%, var(--accent-strong) 0 0.2rem, transparent 0.215rem);
    box-shadow: 0 0 0.5rem var(--accent-glow);
  }

  .disk svg {
    width: 1.25rem;
    height: 1rem;
    display: block;
    fill: none;
    stroke: var(--text-faint);
    stroke-width: 1.05;
    stroke-linecap: round;
  }

  .row.selected .disk svg {
    stroke: var(--text-dim);
  }

  /* The platter is the part that carries the accent: it is the surface the
     scan reads, and on the selected row it is what the eye should land on. */
  .disk .platter {
    stroke-width: 0.95;
  }

  .disk .spindle,
  .disk .pivot {
    fill: var(--text-faint);
    stroke: none;
  }

  .row.selected .disk .platter {
    stroke: var(--accent-strong);
  }

  .row.selected .disk .spindle,
  .row.selected .disk .pivot {
    fill: var(--accent-strong);
  }

  .name {
    font-family: var(--font-mono);
    font-size: 0.84rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind,
  .size {
    font-size: 0.84rem;
    color: var(--text-dim);
  }

  .mounted {
    font-size: 0.72rem;
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: right;
  }

  .mounted.writable {
    color: var(--warn);
  }
</style>
