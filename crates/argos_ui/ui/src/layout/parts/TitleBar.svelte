<script lang="ts">
  /**
   * The window's own title bar: the name on the left, the window buttons on
   * the right, and a drag region across everything between.
   *
   * Where the three buttons are and how big they are is the theme's, because
   * the group belongs to the frame rather than to the content: one generation
   * of window hangs a divided glass strip flush from its top edge, another
   * floats three glyphs beside the title. What no theme decides is that there
   * are three, which three, or their order.
   */
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  import { active } from '../../themes/active.svelte';

  const window = getCurrentWindow();

  /**
   * Whether this window is the one being worked in.
   *
   * A caption says so: on the desktop this theme reproduces, the buttons of a
   * window in the background give up their faces to the glass and the close
   * button gives up its red, because being dangerous is a property of the
   * window under the hands.
   */
  let focused = $state(true);

  onMount(() => {
    let stop: (() => void) | undefined;
    void window.onFocusChanged(({ payload }) => (focused = payload)).then((off) => {
      stop = off;
    });
    return () => stop?.();
  });
</script>

<header data-tauri-drag-region class:inactive={!focused}>
  <span class="name" data-tauri-drag-region>Argos</span>

  <!-- Not a drag region: a drag region takes the press before a button sees
       it, and these three are the buttons a window most needs to work. -->
  <div class="group">
    <button class="window" onclick={() => window.minimize()} aria-label="Minimise">
      <svg viewBox="0 0 12 12" aria-hidden="true">{@html active.icon('minimise')}</svg>
    </button>
    <button class="window" onclick={() => window.toggleMaximize()} aria-label="Maximise">
      <svg viewBox="0 0 12 12" aria-hidden="true">{@html active.icon('maximise')}</svg>
    </button>
    <button class="window close" onclick={() => window.close()} aria-label="Close">
      <svg viewBox="0 0 12 12" aria-hidden="true">{@html active.icon('close')}</svg>
    </button>
  </div>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    height: 2.9rem;
    padding: 0 var(--winbtn-offset-right) 0 1.6rem;
    flex: none;
    user-select: none;
    background: var(--titlebar);
    border-bottom: 1px solid var(--titlebar-border);
  }

  .name {
    font-size: var(--titlebar-text-size);
    font-weight: var(--titlebar-text-weight);
    color: var(--titlebar-text);
    letter-spacing: 0.01em;
    /* Two shadows on a glass theme, none on a flat one: a title over a
       translucent band is legible because it is embossed into it. */
    text-shadow: var(--titlebar-text-shadow, var(--text-glow));
  }

  /* One strip, divided into three, hanging where the theme hangs it. With a
     transparent fill and transparent dividers this is three bare glyphs and
     nothing else — but always these three, always in this order. */
  .group {
    display: flex;
    align-items: stretch;
    gap: var(--winbtn-gap);
    margin-left: auto;
    align-self: var(--winbtn-align);
    margin-top: var(--winbtn-offset-top);
    background: var(--winbtn-group);
    /* Sides only. The line across the top of the group is the window's own
       outer border and the white line under it is the window's inner one —
       the group is flush with the frame's edge and shares both. What closes
       it underneath is each button's own bottom border. */
    border: 0 solid var(--winbtn-group-border);
    border-left-width: 1px;
    border-right-width: 1px;
    border-radius: var(--winbtn-group-radius);
    box-shadow: var(--winbtn-group-shadow);
    overflow: hidden;
  }

  .window {
    display: grid;
    place-items: center;
    width: var(--winbtn-width);
    height: var(--winbtn-height);
    padding: 0;
    background: var(--winbtn-face);
    /* Split, so a theme can say which sides it draws: the caption group of a
       bevelled frame closes its buttons underneath and nowhere else. */
    border: 1px solid;
    border-color: var(--winbtn-border);
    border-radius: var(--winbtn-radius);
    color: var(--winbtn-text);
    cursor: pointer;
  }

  /* The divider is two lines on a theme that bevels — a dark one and the
     light one that catches on it — and nothing at all on one that does not. */
  .window + .window {
    border-left: 1px solid var(--winbtn-divider);
    box-shadow: inset 1px 0 0 var(--winbtn-divider-light);
  }

  .window.close {
    width: var(--winbtn-close-width);
    background: var(--winbtn-close);
    color: var(--winbtn-close-text);
  }

  /* The glyph is the theme's drawing, and it says for itself whether it is a
     line or a filled shape with an outline — the caption glyphs of a bevelled
     desktop are the second, and forcing `fill: none` here made them the
     first. All this decides is how big it is and what colour it inherits. */
  .window svg {
    width: var(--winbtn-glyph);
    height: var(--winbtn-glyph);
    stroke-width: var(--winbtn-stroke);
  }

  .window:hover {
    background: var(--winbtn-hover);
    color: var(--winbtn-hover-text);
  }

  /* On the desktop this theme is ported from, a caption button under the
     pointer does not merely change colour — it throws light below itself. The
     filter sits on the group because the group clips what its children paint;
     a theme with no such light gives it `none`. */
  .group:has(.window:hover) {
    filter: var(--winbtn-hover-filter);
  }

  .group:has(.window.close:hover) {
    filter: var(--winbtn-close-hover-filter);
  }

  .window:active {
    background: var(--winbtn-active);
    color: var(--winbtn-hover-text);
  }

  .window.close:hover {
    background: var(--winbtn-close-hover);
    color: var(--winbtn-close-hover-text);
  }

  .window.close:active {
    background: var(--winbtn-close-active);
    color: var(--winbtn-close-hover-text);
  }

  /* A window in the background, in the terms the reference sets: faces gone,
     edges pale, no red. A theme whose caption does not change says so by
     giving these the same values as the ones above. */
  .inactive .group {
    border-color: var(--winbtn-group-border-off);
  }

  .inactive .window {
    background: var(--winbtn-face-off);
  }

  .inactive .window.close {
    background: var(--winbtn-close-off);
  }

  .inactive .window + .window {
    border-left-color: var(--winbtn-divider-off);
  }

  /* Inside the button rather than around it: the group clips its children, so
     a ring drawn outside the first or last of them would be cut in half. */
  .window:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -2px;
    box-shadow: inset 0 0 0 1px var(--winbtn-divider-light);
  }
</style>
