<script lang="ts">
  /**
   * One progress ring: a label and a percentage inside a circular track.
   *
   * `fraction` is a number the engine reported divided by another number the
   * engine reported. Nothing is inferred here — a ring that cannot be filled
   * because a stage did not say how much work it has stays empty rather than
   * animating to suggest activity it cannot vouch for.
   *
   * The arc is drawn in four passes, which is what gives it depth on a theme
   * that wants depth and costs nothing on one that does not: a darker rim
   * under it, the fill itself, a lighter band along its outer edge, and a soft
   * highlight sweeping the part already filled. A theme that wants a flat arc
   * gives the rim and the band the same colour as the fill and the sweep no
   * duration.
   *
   * The sweep claims nothing: it travels only the filled part, so it says
   * "this much is done, and the run is alive", never "this much more is
   * coming".
   *
   * A `null` fraction is a stage that cannot express itself as one — not a
   * stage at nought. The ring then shows an empty track and a dash in place of
   * a figure, which is the honest reading: the run is going and how far
   * through it is is not a thing that can be said.
   */
  import { percentage } from '../../lib/format';

  let { label, fraction }: { label: string; fraction: number | null } = $props();

  /**
   * Geometry of the arc, in the SVG's own coordinates.
   *
   * The rendered size comes from `--ring-size` instead, so the ring can shrink
   * with the window without the window ever needing to scroll. The nominal
   * stroke below sets the radius; the drawn width is the theme's, and the
   * difference between the two is far too small to move the arc.
   */
  const SIZE = 176;
  const NOMINAL_STROKE = 11;
  const RADIUS = (SIZE - NOMINAL_STROKE) / 2;
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

  /** Length of the sweeping highlight, in the same units as the arc. */
  const PULSE = CIRCUMFERENCE * 0.3;

  /** One filter id per ring, so two on a page do not share one blur. */
  const uid = `ring-${(counter += 1)}`;

  const clamped = $derived(fraction === null ? 0 : Math.min(1, Math.max(0, fraction)));
  const percent = $derived(fraction === null ? null : percentage(fraction));
  const filled = $derived(CIRCUMFERENCE * clamped);
  const offset = $derived(CIRCUMFERENCE - filled);

  /**
   * Whether the sweep has room to travel.
   *
   * Below one pulse-length of filled arc there is nowhere for it to go, and a
   * highlight that sat still would read as a defect rather than as motion.
   */
  const sweeping = $derived(filled > PULSE * 1.2);
</script>

<script lang="ts" module>
  let counter = 0;
</script>

<div class="ring">
  <svg
    viewBox="0 0 {SIZE} {SIZE}"
    role="img"
    aria-label={percent === null ? `${label}, progress not measurable` : `${label} ${percent}%`}
  >
    <defs>
      <filter id={uid} x="-25%" y="-25%" width="150%" height="150%">
        <feGaussianBlur stdDeviation="5" />
      </filter>
    </defs>

    <circle class="track" cx={SIZE / 2} cy={SIZE / 2} r={RADIUS} />
    <circle class="track-edge" cx={SIZE / 2} cy={SIZE / 2} r={RADIUS + NOMINAL_STROKE / 2 - 0.5} />

    {#if clamped > 0}
      <!-- The rim under the fill, a shade darker, so the arc has a body. -->
      <circle
        class="rim"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={RADIUS}
        stroke-dasharray={CIRCUMFERENCE}
        stroke-dashoffset={offset}
      />
      <circle
        class="fill"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={RADIUS}
        stroke-dasharray={CIRCUMFERENCE}
        stroke-dashoffset={offset}
      />
      <!-- The gloss band along the outer edge of the fill. -->
      <circle
        class="gloss"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={RADIUS - 1.6}
        stroke-dasharray={CIRCUMFERENCE}
        stroke-dashoffset={offset}
      />
    {/if}

    {#if sweeping}
      <circle
        class="pulse"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={RADIUS}
        stroke-dasharray="{PULSE} {CIRCUMFERENCE}"
        filter="url(#{uid})"
        style:--sweep-to="{-(filled - PULSE)}"
      />
    {/if}
  </svg>
  <div class="screen" aria-hidden="true"></div>
  <div class="face">
    <span class="label">{label}</span>
    {#if percent === null}
      <span class="value">&mdash;</span>
    {:else}
      <span class="value">{percent}<span class="unit">%</span></span>
    {/if}
  </div>
</div>

<style>
  .ring {
    position: relative;
    width: var(--ring-size, 9.6rem);
    height: var(--ring-size, 9.6rem);
  }

  /* The arc, seen through whatever the theme's surface is made of.
     Masked to the band itself rather than to the square that contains it: an
     unmasked overlay tints its own bounding box, and on a dark theme that box
     is visible as a faint rectangle around each ring. The stops below are the
     stroke's own edges as a fraction of the box — inner edge at 77.25 of 88,
     outer at 87.75 — so the texture starts and ends exactly where the arc
     does. */
  .screen {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background-image: var(--scanlines);
    -webkit-mask-image: radial-gradient(
      closest-side,
      transparent 0 86.5%,
      #000 88.5% 99.5%,
      transparent 100%
    );
    mask-image: radial-gradient(
      closest-side,
      transparent 0 86.5%,
      #000 88.5% 99.5%,
      transparent 100%
    );
  }

  svg {
    width: 100%;
    height: 100%;
    /* Zero degrees at the top, filling clockwise. */
    transform: rotate(-90deg);
    overflow: visible;
  }

  circle {
    fill: none;
    stroke-linecap: var(--ring-cap);
    stroke-width: 10.5;
  }

  .track {
    stroke: var(--track);
  }

  .track-edge {
    stroke: var(--track-edge);
    stroke-width: 1;
  }

  .rim {
    stroke: var(--ring-shadow);
    transition: stroke-dashoffset 240ms ease-out;
  }

  .fill {
    stroke: var(--ring);
    stroke-width: 8.5;
    filter: drop-shadow(0 0 0.28rem var(--ring-glow));
    transition: stroke-dashoffset 240ms ease-out;
  }

  .gloss {
    stroke: var(--ring-highlight);
    stroke-width: 2.6;
    opacity: 0.5;
    transition: stroke-dashoffset 240ms ease-out;
  }

  /* The shine. It travels the filled part of the arc, rests, and goes again.
     Narrower than the fill so it never spills past the band's edges, and
     screened rather than painted on, so it *brightens* the green instead of
     covering it — which is the difference between a highlight and a stripe.
     A theme that wants none gives it no duration, and then it never runs. */
  .pulse {
    stroke: var(--ring-pulse);
    stroke-width: 7;
    mix-blend-mode: screen;
    animation: sweep var(--ring-pulse-duration) linear infinite;
  }

  /* Two thirds travel, one third still: the pause between passes is as much a
     part of this animation as the pass. */
  @keyframes sweep {
    0% {
      stroke-dashoffset: 0;
      opacity: 0;
    }
    10% {
      opacity: 1;
    }
    56% {
      opacity: 1;
    }
    66% {
      stroke-dashoffset: var(--sweep-to);
      opacity: 0;
    }
    100% {
      stroke-dashoffset: var(--sweep-to);
      opacity: 0;
    }
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
    font-size: 0.94rem;
    color: var(--text-dim);
    text-shadow: var(--text-glow);
  }

  .value {
    font-size: 1.9rem;
    text-shadow: var(--text-glow);
    font-weight: 300;
    letter-spacing: -0.02em;
    color: var(--text);
  }

  .unit {
    font-size: 1.05rem;
    color: var(--text-dim);
    margin-left: 0.125rem;
  }
</style>
