<script lang="ts">
  /**
   * The window every dialog in this application is.
   *
   * Not a card that happens to float: the frame, the title strip and the way
   * out are the ones the main window uses, drawn from the same tokens and with
   * the theme's own close glyph. A panel with its own idea of a border and its
   * own X would be a second visual language inside one application, so there is
   * one of these and both dialogs are it.
   *
   * What a caller supplies is the title, how wide the window is, and what goes
   * inside it. Everything else — the scrim, the frame, the bright line inside
   * the dark edge, the strip and the button on it, Escape and a click on the
   * scrim — belongs to the window rather than to what it holds.
   */
  import type { Snippet } from 'svelte';

  import { active } from '../../themes/active.svelte';

  let {
    title,
    width = '44rem',
    onClose,
    children,
  }: {
    /** Named on the strip, and the accessible name of the dialog. */
    title: string;
    /** How wide the window is at most; it never outgrows the viewport. */
    width?: string;
    /** Dismissal: the X, Escape, and a click outside all arrive here. */
    onClose: () => void;
    /** The dialog's own body, laid out by whoever supplies it. */
    children: Snippet;
  } = $props();
</script>

<svelte:window onkeydown={(event) => event.key === 'Escape' && onClose()} />

<div
  class="scrim"
  role="presentation"
  onclick={(event) => event.target === event.currentTarget && onClose()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    style:--dialog-width={width}
  >
    <header>
      <h2>{title}</h2>
      <div class="group">
        <button class="window close" onclick={onClose} aria-label="Close">
          <svg viewBox="0 0 12 12" aria-hidden="true">{@html active.icon('close')}</svg>
        </button>
      </div>
    </header>

    {@render children()}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: var(--scrim);
    backdrop-filter: blur(2px);
    -webkit-backdrop-filter: blur(2px);
  }

  /* Sized against the window rather than in fixed measures: the window can be
     small, and a panel that outgrew it would be a panel with no way out. */
  /*
   * A window, built the way the main one is: the frame is the whole element,
   * the title sits on it, and the panel is inset into it by the width of the
   * band. A theme whose windows have no band gives the inset no width, and the
   * panel then covers everything but the title — which is the same panel it
   * was before.
   *
   * Sized against the window rather than in fixed measures: the window can be
   * small, and a panel that outgrew it would be a panel with no way out.
   */
  .dialog {
    position: relative;
    display: flex;
    flex-direction: column;
    width: min(var(--dialog-width), calc(100vw - 3rem));
    max-height: calc(100vh - 3rem);
    background: var(--dialog-strip);
    backdrop-filter: var(--dialog-blur);
    -webkit-backdrop-filter: var(--dialog-blur);
    border: 1px solid var(--window-edge);
    border-radius: var(--window-radius);
    box-shadow: var(--window-shadow);
    padding: 0;
    overflow: hidden;
  }

  /* The same bright line inside the dark edge that the main window has. */
  .dialog::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 3;
    pointer-events: none;
    border-radius: inherit;
    box-shadow: inset 0 0 0 1px var(--window-border);
  }

  /*
   * The title strip, and the one piece of glass in this application that is
   * real.
   *
   * The window's own frame lies over the desktop, which no filter in a web
   * view can reach; this lies over the window, which *is* the page — so the
   * blur behind it blurs something, and on a theme whose windows are framed in
   * glass this is that glass, measured on the same profile as the frame. On a
   * theme whose window is one unbroken surface it is the panel's own fill and
   * the panel's own filter, and there is no strip to see.
   */
  header {
    display: flex;
    align-items: center;
    flex: none;
    height: var(--dialog-strip-height);
    padding: 0 var(--winbtn-offset-right) 0 1.25rem;
  }

  h2 {
    margin: 0;
    font-size: var(--type-md);
    font-weight: 600;
    color: var(--titlebar-text);
    text-shadow: var(--titlebar-text-shadow, var(--text-glow));
  }

  /* The same strip the window buttons sit in, holding the one button a panel
     needs. */
  .group {
    display: flex;
    align-items: stretch;
    gap: var(--winbtn-gap);
    margin-left: auto;
    /* The group hangs from the band's top edge on a theme whose windows do,
       and sits on the title's own line on one whose windows do not. */
    align-self: var(--winbtn-align);
    margin-top: var(--winbtn-offset-top);
    background: var(--winbtn-group);
    border: 1px solid var(--winbtn-group-border);
    border-top: 0;
    border-radius: var(--winbtn-group-radius);
    box-shadow: var(--winbtn-group-shadow);
    overflow: hidden;
  }

  .window {
    display: grid;
    place-items: center;
    width: var(--winbtn-close-width);
    height: var(--winbtn-height);
    padding: 0;
    background: var(--winbtn-close);
    border: 1px solid;
    border-color: var(--winbtn-border);
    border-radius: var(--winbtn-radius);
    color: var(--winbtn-close-text);
    cursor: pointer;
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

  .window.close:hover {
    background: var(--winbtn-close-hover);
    color: var(--winbtn-close-hover-text);
  }

  .window.close:active {
    background: var(--winbtn-close-active);
    color: var(--winbtn-close-hover-text);
  }

  .window:focus-visible {
    outline: var(--focus-outline);
    outline-offset: -2px;
  }
</style>
