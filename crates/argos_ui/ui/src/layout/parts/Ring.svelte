<script lang="ts">
  /**
   * One progress ring: a label and a percentage inside a circular groove.
   *
   * `fraction` is a number the engine reported divided by another number the
   * engine reported. Nothing is inferred here — a ring that cannot be filled
   * because a stage did not say how much work it has stays empty rather than
   * animating to suggest activity it cannot vouch for.
   *
   * A `null` fraction is a stage that cannot express itself as one — not a
   * stage at nought. The ring then shows an empty groove and a dash in place
   * of a figure, which is the honest reading: the run is going and how far
   * through it is is not a thing that can be said.
   *
   * **The arc is a bar bent into a circle, and it is built the way a bar of
   * that kind is built.** A bar is shaded across its height: light along one
   * edge, saturated through the middle, brightening again at the other edge,
   * with a line of unequal weight closing each side. Bent round, "across its
   * height" becomes "across the band", so every one of those layers is a
   * concentric stroke here rather than a gradient stop — six of them, drawn
   * from the groove outwards:
   *
   * 1. the groove's own channel: a light band along its near edge, its body,
   *    and a lighter one along its far edge;
   * 2. the fill;
   * 3. the light band along the fill's inner edge;
   * 4. the brighter band along its outer edge;
   * 5. the sweep, travelling the filled part;
   * 6. the two lines that close the groove, drawn last because they close over
   *    the fill as well — a groove does not stop where its fill starts.
   *
   * The sweep claims nothing: it travels only the filled part, so it says
   * "this much is done, and the run is alive", never "this much more is
   * coming".
   */
  import { percentage } from '../../lib/format';

  let { label, fraction }: { label: string; fraction: number | null } = $props();

  /**
   * Geometry of the arc, in the SVG's own coordinates.
   *
   * The rendered size comes from `--ring-size` instead, so the ring can shrink
   * with the window without the window ever needing to scroll.
   */
  const SIZE = 176;
  const BAND = 10.5;
  const RADIUS = (SIZE - BAND - 0.5) / 2;
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

  /**
   * The two bands inside the fill, as fractions of the band's width, and where
   * their centres sit across it.
   *
   * Measured off the reference bar: its first fifth is the light band and its
   * last fifth the bright one, with three fifths of flat colour between them.
   */
  const LIT = BAND * 0.22;
  const EDGE = BAND * 0.2;
  const LIT_RADIUS = RADIUS - BAND / 2 + LIT / 2;
  const EDGE_RADIUS = RADIUS + BAND / 2 - EDGE / 2;

  /** Where the two lines that close the groove sit. */
  const INNER_EDGE = RADIUS - BAND / 2 + 0.5;
  const OUTER_EDGE = RADIUS + BAND / 2 - 0.5;

  /**
   * How much of the arc the sweep covers, and how long it rests between
   * passes.
   *
   * A third of the arc is what a bar of this kind used, and it is the layout's
   * measure rather than a theme's: a sweep half the arc long stops reading as
   * a highlight travelling and starts reading as the arc changing colour.
   */
  const SHEEN = CIRCUMFERENCE * 0.33;

  /** One filter id per ring, so two on a page do not share one blur. */
  const uid = `ring-${(counter += 1)}`;

  const clamped = $derived(fraction === null ? 0 : Math.min(1, Math.max(0, fraction)));
  const percent = $derived(fraction === null ? null : percentage(fraction));
  const filled = $derived(CIRCUMFERENCE * clamped);
  const offset = $derived(CIRCUMFERENCE - filled);

  /**
   * Whether the sweep has room to travel.
   *
   * Below one sweep-length of filled arc there is nowhere for it to go, and a
   * highlight that sat still would read as a defect rather than as motion.
   */
  const sweeping = $derived(filled > SHEEN * 1.2);
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
        <feGaussianBlur stdDeviation="6" />
      </filter>
    </defs>

    <circle class="track" cx={SIZE / 2} cy={SIZE / 2} r={RADIUS} stroke-width={BAND} />
    <circle class="track-lit" cx={SIZE / 2} cy={SIZE / 2} r={LIT_RADIUS} stroke-width={LIT} />
    <circle class="track-far" cx={SIZE / 2} cy={SIZE / 2} r={EDGE_RADIUS} stroke-width={EDGE} />

    {#if clamped > 0}
      <circle
        class="fill"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={RADIUS}
        stroke-width={BAND}
        stroke-dasharray={CIRCUMFERENCE}
        stroke-dashoffset={offset}
      />
      <circle
        class="lit"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={LIT_RADIUS}
        stroke-width={LIT}
        stroke-dasharray={2 * Math.PI * LIT_RADIUS}
        stroke-dashoffset={2 * Math.PI * LIT_RADIUS * (1 - clamped)}
      />
      <circle
        class="edge"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={EDGE_RADIUS}
        stroke-width={EDGE}
        stroke-dasharray={2 * Math.PI * EDGE_RADIUS}
        stroke-dashoffset={2 * Math.PI * EDGE_RADIUS * (1 - clamped)}
      />
    {/if}

    {#if sweeping}
      <circle
        class="sweep"
        cx={SIZE / 2}
        cy={SIZE / 2}
        r={RADIUS}
        stroke-width={BAND - 2}
        stroke-dasharray="{SHEEN} {CIRCUMFERENCE}"
        filter="url(#{uid})"
        style:--sweep-to="{-(filled - SHEEN)}"
      />
    {/if}

    <circle class="groove-inner" cx={SIZE / 2} cy={SIZE / 2} r={INNER_EDGE} />
    <circle class="groove-outer" cx={SIZE / 2} cy={SIZE / 2} r={OUTER_EDGE} />
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
     is visible as a faint rectangle around each ring. */
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
  }

  .track {
    stroke: var(--track);
  }

  /* The channel is not one flat colour: it is light where it turns towards the
     light and darker at its deepest, which on a ring runs across the band
     rather than down it. The far edge is between the two, and is mixed here
     rather than named by the theme so that a theme with one colour keeps one
     colour. */
  .track-lit {
    stroke: var(--track-lit);
  }

  .track-far {
    stroke: color-mix(in srgb, var(--track-lit) 55%, var(--track));
  }

  .fill {
    stroke: var(--ring);
    filter: drop-shadow(0 0 0.24rem var(--ring-glow));
    transition: stroke-dashoffset 240ms ease-out;
  }

  .lit {
    stroke: var(--ring-highlight);
    transition: stroke-dashoffset 240ms ease-out;
  }

  .edge {
    stroke: var(--ring-edge);
    transition: stroke-dashoffset 240ms ease-out;
  }

  /* The two lines that close the channel, over everything: unequal, because a
     groove is deeper on one side than the other. */
  .groove-inner {
    stroke: color-mix(in srgb, var(--track-edge) 55%, var(--track));
    stroke-width: 1;
  }

  .groove-outer {
    stroke: var(--track-edge);
    stroke-width: 1;
  }

  /* The sweep. It crosses the filled part, rests, and goes again — eased at
     both ends, because what separates this from a shimmer is the easing and
     the opacity rather than the speed. Blurred so it has no edges of its own,
     and screened rather than painted on, so it brightens the arc instead of
     covering it. A theme that wants none gives it no duration, and then it
     never runs. */
  .sweep {
    stroke: var(--sheen);
    mix-blend-mode: screen;
    animation: sweep var(--sheen-duration) var(--sheen-easing) infinite;
  }

  /* Two thirds travel, one third still: the pause between passes is as much a
     part of this animation as the pass. */
  @keyframes sweep {
    0% {
      stroke-dashoffset: 0;
      opacity: 0;
    }
    12% {
      opacity: 1;
    }
    54% {
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

  /* A viewer who asked for less motion gets the figure and the arc, which say
     everything the sweep says except that the run is alive — and the elapsed
     clock beside them says that. */
  @media (prefers-reduced-motion: reduce) {
    .sweep {
      display: none;
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
    font-size: var(--type-lg);
    color: var(--text-dim);
    text-shadow: var(--text-glow);
  }

  .value {
    font-size: var(--type-3xl);
    text-shadow: var(--text-glow);
    font-weight: 300;
    letter-spacing: -0.02em;
    color: var(--text);
  }

  .unit {
    font-size: var(--type-xl);
    color: var(--text-dim);
    margin-left: 0.125rem;
  }
</style>
