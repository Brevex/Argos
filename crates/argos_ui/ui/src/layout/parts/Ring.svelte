<script lang="ts">
  /**
   * One progress ring: a label and a percentage inside a circular track.
   *
   * `fraction` is a number the engine reported divided by another number the
   * engine reported. Nothing is inferred here — a ring that cannot be filled
   * because a stage did not say how much work it has stays empty rather than
   * animating to suggest activity it cannot vouch for.
   */
  import { percentage } from '../../lib/format';

  let { label, fraction }: { label: string; fraction: number } = $props();

  /**
   * Geometry of the arc, in the SVG's own coordinates.
   *
   * The rendered size comes from `--ring-size` instead, so the ring can shrink
   * with the window without the window ever needing to scroll.
   */
  const SIZE = 176;
  const STROKE = 11;
  const RADIUS = (SIZE - STROKE) / 2;
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

  const percent = $derived(percentage(fraction));
  const offset = $derived(CIRCUMFERENCE * (1 - Math.min(1, Math.max(0, fraction))));
</script>

<div class="ring">
  <svg viewBox="0 0 {SIZE} {SIZE}" role="img" aria-label="{label} {percent}%">
    <circle
      class="track"
      cx={SIZE / 2}
      cy={SIZE / 2}
      r={RADIUS}
      stroke-width={STROKE}
    />
    <circle
      class="fill"
      class:lit={percent > 0}
      cx={SIZE / 2}
      cy={SIZE / 2}
      r={RADIUS}
      stroke-width={STROKE}
      stroke-dasharray={CIRCUMFERENCE}
      stroke-dashoffset={offset}
    />
  </svg>
  <div class="face">
    <span class="label">{label}</span>
    <span class="value">{percent}<span class="unit">%</span></span>
  </div>
</div>

<style>
  .ring {
    position: relative;
    width: var(--ring-size, 11rem);
    height: var(--ring-size, 11rem);
  }

  svg {
    width: 100%;
    height: 100%;
    /* Zero degrees at the top, filling clockwise. */
    transform: rotate(-90deg);
  }

  circle {
    fill: none;
    stroke-linecap: round;
  }

  .track {
    stroke: var(--track);
  }

  .fill {
    stroke: var(--ring);
    transition: stroke-dashoffset 240ms ease-out;
  }

  .fill.lit {
    filter: drop-shadow(0 0 0.375rem var(--ring-glow));
  }

  .face {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.125rem;
  }

  .label {
    font-size: 0.91rem;
    color: var(--text-dim);
  }

  .value {
    font-size: 2rem;
    font-weight: 300;
    letter-spacing: -0.02em;
    color: var(--text);
  }

  .unit {
    font-size: 1.15rem;
    color: var(--text-dim);
    margin-left: 0.125rem;
  }
</style>
