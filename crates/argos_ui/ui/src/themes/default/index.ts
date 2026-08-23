import { theme } from '../contract';

/**
 * The theme the window opens with: layered glass over a dark ground.
 *
 * The grammar here is the opposite of a bevelled desktop's. Depth is made of
 * **layers, not edges**: a surface sits above another because it is a little
 * lighter, a little more translucent and edge-lit by a border a shade brighter
 * than its own fill — never because it has a light line on top and a dark one
 * underneath. Nothing is embossed, nothing is glossed, and there is no
 * specular break down the middle of anything.
 *
 * Colour is spent almost entirely on one accent: the chosen drive, the arcs
 * and the button that starts the work. Everything else is greyscale, so that
 * on a screen full of chrome the saturated things are the ones that mean
 * something. Even the close button keeps to it — closing a window is not an
 * emergency, and a red that appears nowhere else in this theme would be the
 * loudest thing on screen.
 */
export default theme({
  id: 'default',
  name: 'Default',
  scheme: 'dark',
  controls: { checkbox: 'switch', choice: 'dot' },
  tokens: {
    // ---------------------------------------------------------------- frame
    '--backdrop': '#14171d',
    // A wide, very dim pool of light behind the window, so the translucent
    // panes have something to be translucent against.
    '--backdrop-glow':
      'radial-gradient(120% 90% at 50% 0%, rgba(58, 88, 140, 0.16), transparent 62%), ' +
      'radial-gradient(90% 70% at 50% 108%, rgba(40, 62, 104, 0.12), transparent 60%)',
    '--backdrop-noise': 'none',

    '--window-edge': 'rgba(0, 0, 0, 0.55)',
    '--window-border': 'rgba(202, 208, 217, 0.16)',
    '--window-radius': '0.78rem',
    '--window-shadow': '0 1.2rem 3rem -1.4rem rgba(0, 0, 0, 0.82)',
    '--window-inset': '0rem',

    '--main-surface': 'transparent',
    '--main-border': 'transparent',
    '--main-shadow': 'none',
    '--main-radius': '0',

    '--titlebar': 'transparent',
    '--titlebar-border': 'transparent',
    '--titlebar-text': '#eef1f5',
    '--titlebar-text-size': '1.06rem',
    '--titlebar-text-weight': '600',
    '--titlebar-text-shadow': 'none',

    // ------------------------------------------------------- caption group
    // Three glyphs beside the title, each its own target: no strip, no
    // dividers, and a corner soft enough to belong to this window without
    // becoming a pill.
    '--winbtn-align': 'center',
    '--winbtn-offset-top': '0rem',
    '--winbtn-offset-right': '0.75rem',
    '--winbtn-height': '1.7rem',
    '--winbtn-width': '2.4rem',
    '--winbtn-close-width': '2.4rem',
    '--winbtn-glyph': '0.8rem',
    // Three separate targets, each with the same hairline every other surface
    // in this theme is edged with, and air between them.
    '--winbtn-border': 'rgba(255, 255, 255, 0.14)',
    '--winbtn-radius': '0.4rem',
    '--winbtn-gap': '0.3rem',
    '--winbtn-group-pull': '0rem',
    '--winbtn-group': 'transparent',
    '--winbtn-group-border': 'transparent',
    '--winbtn-group-radius': '0.45rem',
    '--winbtn-group-shadow': 'none',
    // The line between two buttons is, on a theme that gives each of them its
    // own edge, that edge — not a divider laid over it. Left transparent it
    // ate the left border of every button after the first.
    '--winbtn-divider': 'rgba(255, 255, 255, 0.14)',
    '--winbtn-divider-light': 'transparent',
    '--winbtn-face': 'transparent',
    '--winbtn-text': '#a7b0bd',
    '--winbtn-hover': 'rgba(255, 255, 255, 0.08)',
    // No glow under the pointer: this caption is a change of fill, not of light.
    '--winbtn-hover-filter': 'none',
    '--winbtn-close-hover-filter': 'none',
    '--winbtn-hover-text': '#eef1f5',
    '--winbtn-active': 'rgba(255, 255, 255, 0.14)',
    '--winbtn-close': 'transparent',
    '--winbtn-close-text': '#a7b0bd',
    '--winbtn-close-hover': 'rgba(255, 255, 255, 0.12)',
    '--winbtn-close-hover-text': '#eef1f5',
    '--winbtn-close-active': 'rgba(255, 255, 255, 0.18)',
    '--winbtn-stroke': '1.2',
    // This theme's caption is the same whether the window has the focus or
    // not: there is no face to give up and no red to withdraw.
    '--winbtn-face-off': 'transparent',
    '--winbtn-close-off': 'transparent',
    '--winbtn-group-border-off': 'transparent',
    '--winbtn-divider-off': 'rgba(255, 255, 255, 0.14)',

    // ---------------------------------------------------------------- panes
    '--pane': 'rgba(255, 255, 255, 0.045)',
    '--pane-blur': 'blur(24px) saturate(140%)',
    '--pane-border': 'rgba(255, 255, 255, 0.09)',
    '--pane-shadow':
      'inset 0 1px 0 rgba(255, 255, 255, 0.06), 0 1.125rem 3rem -1.5rem rgba(0, 0, 0, 0.9)',
    '--pane-radius': '0.7rem',
    '--form-pane': 'rgba(24, 28, 34, 0.88)',
    '--form-pane-border': 'rgba(205, 211, 220, 0.19)',
    '--form-pane-shadow': 'none',
    // The panel's own fill and the panel's own blur: a settings window here is
    // one unbroken surface, and a band across its top would be the only band
    // in the whole interface.
    '--dialog-strip': 'rgba(255, 255, 255, 0.045)',
    '--dialog-strip-height': '2.2rem',
    // No frame: the panel is the whole window.
    '--dialog-inset': '0rem',
    '--dialog-blur': 'blur(24px) saturate(140%)',
    '--scrim': 'rgba(4, 5, 7, 0.62)',

    // ----------------------------------------------------------------- rows
    '--row': 'transparent',
    '--row-hover': 'rgba(255, 255, 255, 0.055)',
    '--row-selected': 'rgba(47, 124, 246, 0.17)',
    '--row-selected-border': 'rgba(96, 160, 255, 0.55)',
    '--row-selected-shadow': 'inset 0 1px 0 rgba(255, 255, 255, 0.05)',
    '--row-radius': '0.42rem',

    '--choice': 'transparent',
    '--choice-border': '#6d7784',
    '--choice-border-selected': '#4d94ff',
    '--choice-mark': 'radial-gradient(circle at 50% 50%, #4d94ff 0 55%, transparent 58%)',
    '--choice-shadow': 'none',

    // --------------------------------------------------------------- fields
    '--inset': 'rgba(0, 0, 0, 0.28)',
    '--inset-border': 'rgba(255, 255, 255, 0.08)',
    '--inset-border-hover': 'rgba(255, 255, 255, 0.16)',
    '--inset-shadow': 'none',
    '--input-radius': '0.5rem',
    '--track': 'rgba(255, 255, 255, 0.07)',
    '--track-edge': 'rgba(255, 255, 255, 0.12)',
    '--track-lit': 'rgba(255, 255, 255, 0.11)',
    '--track-shadow': 'none',

    // -------------------------------------------------------------- buttons
    '--action': 'linear-gradient(180deg, #4d94ff, #2f7cf6)',
    '--action-hover': 'linear-gradient(180deg, #63a3ff, #3f88f8)',
    '--action-active': 'linear-gradient(180deg, #2f7cf6, #2569d4)',
    '--action-text': '#ffffff',
    '--action-text-hover': '#ffffff',
    '--action-border': 'rgba(96, 160, 255, 0.55)',
    '--action-shadow': '0 0.375rem 1.125rem -0.5rem rgba(47, 124, 246, 0.42)',
    '--action-shadow-active': 'inset 0 1px 3px rgba(0, 0, 0, 0.3)',
    '--button': 'rgba(255, 255, 255, 0.06)',
    '--button-hover': 'rgba(255, 255, 255, 0.1)',
    '--button-active': 'rgba(255, 255, 255, 0.03)',
    '--button-text': '#dfe4ea',
    '--button-text-hover': '#ffffff',
    '--button-border': 'rgba(255, 255, 255, 0.12)',
    '--button-border-hover': 'rgba(255, 255, 255, 0.22)',
    '--button-shadow': 'none',
    '--button-shadow-active': 'inset 0 1px 3px rgba(0, 0, 0, 0.35)',
    '--button-radius': '0.45rem',
    '--link': '#4d94ff',
    '--link-hover': '#7db1ff',

    '--disabled-text': '#6d7784',
    '--disabled-opacity': '0.45',

    '--focus-outline': '2px solid #4d94ff',
    '--focus-offset': '2px',

    // ------------------------------------------------------------- progress
    '--ring': '#2f7cf6',
    '--ring-highlight': 'color-mix(in srgb, var(--ring) 62%, #ffffff)',
    '--ring-edge': 'color-mix(in srgb, var(--ring) 78%, #0a0d14)',
    '--ring-glow': 'color-mix(in srgb, var(--ring) 38%, transparent)',
    '--ring-cap': 'round',
    '--progress-running': '#2f7cf6',
    '--progress-paused': '#d9a441',
    '--progress-done': '#3fb950',
    '--progress-cancelled': '#8b949e',
    '--progress-failed': '#e5534b',
    // Light travelling under glass rather than a gloss sliding over it: wide,
    // faint, and slower than the bevelled theme's.
    '--sheen': 'rgba(255, 255, 255, 0.34)',
    '--sheen-duration': '2.6s',
    '--sheen-easing': 'cubic-bezier(0.45, 0, 0.55, 1)',

    // ------------------------------------------------------------- controls
    // A pill, because everything else here is soft-cornered.
    '--switch-track': 'rgba(255, 255, 255, 0.12)',
    '--switch-track-on': '#4d94ff',
    '--switch-border': 'rgba(255, 255, 255, 0.16)',
    '--switch-border-on': '#4d94ff',
    '--switch-thumb': '#eef1f5',
    '--switch-radius': '999px',
    '--switch-thumb-radius': '50%',
    '--check-box': 'rgba(255, 255, 255, 0.06)',
    '--check-box-checked': '#2f7cf6',
    '--check-border': 'rgba(255, 255, 255, 0.2)',
    '--check-border-checked': '#4d94ff',
    '--check-mark': '#ffffff',
    '--check-radius': '0.3rem',
    '--check-shadow': 'none',
    '--check-border-hover': 'rgba(255, 255, 255, 0.28)',
    '--check-shadow-hover': 'none',

    '--badge': 'rgba(217, 164, 65, 0.14)',
    '--badge-border': 'rgba(217, 164, 65, 0.4)',
    '--badge-text': '#e3b866',
    '--badge-quiet': 'rgba(255, 255, 255, 0.05)',
    '--badge-quiet-border': 'rgba(255, 255, 255, 0.12)',
    '--badge-quiet-text': '#9aa4b1',
    '--badge-radius': '0.35rem',

    '--scrollbar-track': 'transparent',
    '--scrollbar-thumb': 'rgba(255, 255, 255, 0.14)',
    '--scrollbar-thumb-hover': 'rgba(255, 255, 255, 0.26)',
    '--scrollbar-border': 'transparent',
    '--scrollbar-radius': '99px',

    // --------------------------------------------------------------- ground
    '--scanlines': 'none',
    '--text-glow': 'none',
    '--bevel-raised': 'none',
    '--bevel-sunken': 'none',
    // Not a gloss with a break in it: one soft fall of light down the top of
    // a surface, which is what glass in this idiom does.
    '--specular':
      'linear-gradient(180deg, rgba(255, 255, 255, 0.07) 0%, rgba(255, 255, 255, 0.02) 40%, ' +
      'rgba(255, 255, 255, 0) 72%)',

    '--text': '#eef1f5',
    '--text-dim': '#a7b0bd',
    '--text-faint': '#6d7784',
    '--heading': '#c8d0da',

    '--accent': '#2f7cf6',
    '--accent-strong': '#4d94ff',
    '--accent-text': '#ffffff',
    '--ok': '#3fb950',
    '--warn': '#d9a441',
    '--danger': '#e5534b',

    '--font': '"Adwaita Sans", "Noto Sans", "Liberation Sans", sans-serif',
  },
  icons: {
    minimise: '<path d="M2.5 6h7" fill="none" stroke="currentColor" stroke-linecap="round"/>',
    maximise:
      '<rect x="2.5" y="2.5" width="7" height="7" rx="1.4" fill="none" stroke="currentColor"/>',
    close:
      '<path d="M2.8 2.8l6.4 6.4M9.2 2.8l-6.4 6.4" fill="none" stroke="currentColor" ' +
      'stroke-linecap="round"/>',
    tick:
      '<path d="M2.4 7.4l3.2 3.2 6-6.8" fill="none" stroke="currentColor" stroke-width="1.6" ' +
      'stroke-linecap="round" stroke-linejoin="round"/>',
    up: '<path d="M1.2 4.4 4 1.6l2.8 2.8" fill="none" stroke="currentColor" stroke-width="1.3" ' +
      'stroke-linecap="round" stroke-linejoin="round"/>',
    down: '<path d="M1.2 1.6 4 4.4l2.8-2.8" fill="none" stroke="currentColor" stroke-width="1.3" ' +
      'stroke-linecap="round" stroke-linejoin="round"/>',
  },
});
