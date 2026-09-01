import { theme } from '../contract';

/**
 * Retro: a green phosphor terminal of the mainframe era.
 *
 * One colour, one typeface, square corners and hairline rules. There is no
 * gradient anywhere, nothing is translucent and nothing casts a shadow,
 * because a character display had no depth to represent: a cell was lit or it
 * was not, and brightness does all the work that hue and elevation do
 * elsewhere.
 *
 * The grammar is **inversion**. A terminal could not shade a control to say it
 * was under the pointer, so it swapped the ink and the ground — and that is
 * what hover and press do here. A control that is on is a lit cell inside
 * brackets; one that is chosen is a filled cell. Nothing rounds, nothing
 * glows except the phosphor itself.
 *
 * Two things are allowed to break that flatness, and only two. The progress
 * arcs glow, the way a phosphor dot actually did. And a grid of unlit lines
 * lies over the lit surfaces, so they read as something drawn on cells rather
 * than on pixels.
 */
export default theme({
  id: 'retro',
  name: 'Retro',
  scheme: 'dark',
  // Brackets: the only mark a character display had for either question.
  controls: { checkbox: 'bracket', choice: 'bracket' },
  tokens: {
    // frame
    '--backdrop': '#030b05',
    '--backdrop-glow': 'none',
    '--backdrop-noise': 'none',

    '--window-edge': '#3ca84c',
    '--window-border': 'transparent',
    '--window-radius': '0',
    '--window-shadow': 'none',
    '--window-inset': '0rem',

    '--main-surface': 'transparent',
    '--main-border': 'transparent',
    '--main-shadow': 'none',
    '--main-radius': '0',

    '--titlebar': 'transparent',
    '--titlebar-border': '#3ca84c',
    '--titlebar-text': '#6dff6d',
    '--titlebar-text-size': '1.06rem',
    '--titlebar-text-weight': '600',
    '--titlebar-text-shadow': '0 0 0.42em rgba(104, 240, 104, 0.5)',

    // caption group
    // Three bare glyphs on the character grid, each one cell wide, with no
    // strip and no dividers — all a terminal would have drawn.
    '--winbtn-align': 'center',
    '--winbtn-offset-top': '0rem',
    '--winbtn-offset-right': '0.75rem',
    '--winbtn-height': '1.7rem',
    '--winbtn-width': '2.4rem',
    '--winbtn-close-width': '2.4rem',
    '--winbtn-glyph': '0.8rem',
    '--winbtn-border': 'transparent',
    '--winbtn-radius': '0',
    '--winbtn-gap': '0rem',
    '--winbtn-group-pull': '0rem',
    '--winbtn-group': 'transparent',
    '--winbtn-group-border': 'transparent',
    '--winbtn-group-radius': '0',
    '--winbtn-group-shadow': 'none',
    '--winbtn-divider': 'transparent',
    '--winbtn-divider-light': 'transparent',
    '--winbtn-face': 'transparent',
    '--winbtn-text': '#6dff6d',
    // Inversion, not a wash: ink and ground swap.
    '--winbtn-hover': '#6dff6d',
    // No glow under the pointer: this caption is a change of fill, not of light.
    '--winbtn-hover-filter': 'none',
    '--winbtn-close-hover-filter': 'none',
    '--winbtn-hover-text': '#02150a',
    '--winbtn-active': '#a6ffa6',
    '--winbtn-close': 'transparent',
    '--winbtn-close-text': '#6dff6d',
    '--winbtn-close-hover': '#ff5a5a',
    '--winbtn-close-hover-text': '#02150a',
    '--winbtn-close-active': '#ff8f8f',
    '--winbtn-stroke': '1.2',
    // This theme's caption is the same whether the window has the focus or
    // not: there is no face to give up and no red to withdraw.
    '--winbtn-face-off': 'transparent',
    '--winbtn-close-off': 'transparent',
    '--winbtn-group-border-off': 'transparent',
    '--winbtn-divider-off': 'transparent',

    // panes
    // A pane is the same ground as the window with a rule drawn around it.
    '--pane': '#030b05',
    '--pane-blur': 'none',
    '--pane-border': '#3ca84c',
    '--pane-shadow': 'none',
    '--pane-radius': '0',
    '--form-pane': 'transparent',
    '--form-pane-border': '#3ca84c',
    '--form-pane-shadow': 'none',
    '--dialog-strip': '#030b05',
    '--dialog-strip-height': '2.2rem',
    // No frame: the panel is the whole window.
    '--dialog-inset': '0rem',
    '--dialog-blur': 'none',
    '--scrim': 'rgba(3, 11, 5, 0.72)',

    // rows
    '--row': 'transparent',
    '--row-hover': '#0b2211',
    '--row-selected': '#0f3018',
    '--row-selected-border': '#4ee44e',
    '--row-selected-shadow': 'none',
    '--row-radius': '0',

    '--choice': 'transparent',
    '--choice-border': 'transparent',
    '--choice-border-selected': 'transparent',
    '--choice-mark': '#6dff6d',
    '--choice-shadow': 'none',

    // fields
    '--inset': '#020803',
    '--inset-border': '#3ca84c',
    '--inset-border-hover': '#6dff6d',
    '--inset-shadow': 'none',
    '--input-radius': '0',
    '--track': '#0d2c14',
    '--track-edge': '#24682f',
    '--track-lit': '#24682f',
    '--track-shadow': 'none',

    // buttons
    // Outlined, not filled: the border and the label carry it, the way a
    // terminal drew a control it could not shade. Hover inverts.
    '--action': 'transparent',
    '--action-hover': '#6dff6d',
    '--action-active': '#a6ffa6',
    '--action-text': '#6dff6d',
    '--action-text-hover': '#02150a',
    '--action-border': '#4ee44e',
    '--action-shadow': 'none',
    '--action-shadow-active': 'none',
    '--button': 'transparent',
    '--button-hover': '#6dff6d',
    '--button-active': '#a6ffa6',
    '--button-text': '#5cf85c',
    '--button-text-hover': '#02150a',
    '--button-border': '#3ca84c',
    '--button-border-hover': '#6dff6d',
    '--button-shadow': 'none',
    '--button-shadow-active': 'none',
    '--button-radius': '0',
    '--link': '#6dff6d',
    '--link-hover': '#a6ffa6',

    '--disabled-text': '#256b28',
    '--disabled-opacity': '1',

    // A block cursor's worth of edge, in the one colour there is.
    '--focus-outline': '1px solid #a6ffa6',
    '--focus-offset': '1px',

    // progress
    '--ring': '#68f068',
    '--ring-highlight': 'var(--ring)',
    '--ring-edge': 'var(--ring)',
    '--ring-glow': 'color-mix(in srgb, var(--ring) 55%, transparent)',
    // Flat ends. A rounded cap is a shape a cell grid cannot produce.
    '--ring-cap': 'butt',
    '--progress-running': '#68f068',
    '--progress-paused': '#c8d94a',
    '--progress-done': '#a6ffa6',
    '--progress-cancelled': '#309e30',
    '--progress-failed': '#ff5a5a',
    // A phosphor display had no travelling gloss, and inventing one would be
    // the one anachronism this theme cannot absorb.
    '--sheen': 'transparent',
    '--sheen-duration': '0s',
    '--sheen-easing': 'linear',

    // controls
    '--switch-track': '#020803',
    '--switch-track-on': '#0b3a12',
    '--switch-border': '#3ca84c',
    '--switch-border-on': '#6dff6d',
    '--switch-thumb': '#6dff6d',
    '--switch-radius': '0',
    '--switch-thumb-radius': '0',
    // The idiom this theme uses: a bracketed cell, lit or unlit.
    '--check-box': 'transparent',
    '--check-box-checked': 'transparent',
    '--check-border': '#3ca84c',
    '--check-border-checked': '#6dff6d',
    '--check-mark': '#6dff6d',
    '--check-radius': '0',
    '--check-shadow': 'none',
    '--check-border-hover': '#6dff6d',
    '--check-shadow-hover': 'none',

    '--badge': 'transparent',
    '--badge-border': '#c8d94a',
    '--badge-text': '#c8d94a',
    '--badge-quiet': 'transparent',
    '--badge-quiet-border': '#256b28',
    '--badge-quiet-text': '#43cc43',
    '--badge-radius': '0',

    '--scrollbar-track': '#020803',
    '--scrollbar-thumb': '#24682f',
    '--scrollbar-thumb-hover': '#6dff6d',
    '--scrollbar-border': '#3ca84c',
    '--scrollbar-radius': '0',

    // ground
    // Rows, and only rows: a display drew in lines, so the texture is lines —
    // one unlit row in every two, at the smallest pitch that survives being
    // drawn. It goes on lit surfaces and on nothing else, so the frame, the
    // title and the window edge stay as sharp as they were.
    '--scanlines':
      'repeating-linear-gradient(180deg, rgba(0, 0, 0, 0.32) 0 1px, transparent 1px 2px)',
    // The bloom around a lit phosphor dot. This, rather than any grid, is what
    // makes green text read as a screen instead of as green text.
    '--text-glow': '0 0 0.42em rgba(104, 240, 104, 0.5)',
    '--bevel-raised': 'none',
    '--bevel-sunken': 'none',
    '--specular': 'none',

    '--text': '#5cf85c',
    '--text-dim': '#43cc43',
    '--text-faint': '#309e30',
    '--heading': '#6dff6d',

    '--accent': '#3ad13a',
    '--accent-strong': '#6dff6d',
    '--accent-text': '#02150a',
    '--ok': '#4ee44e',
    '--warn': '#c8d94a',
    '--danger': '#ff5a5a',

    '--font': '"Adwaita Mono", "Liberation Mono", "Noto Sans Mono", "Nimbus Mono PS", monospace',
  },
  icons: {
    minimise:
      '<path d="M2 8h8" fill="none" stroke="currentColor" stroke-linecap="square" ' +
      'shape-rendering="crispEdges"/>',
    maximise:
      '<rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" ' +
      'shape-rendering="crispEdges"/>',
    close:
      '<path d="M2.6 2.6l6.8 6.8M9.4 2.6l-6.8 6.8" fill="none" stroke="currentColor" ' +
      'stroke-linecap="square"/>',
    // A cell, filled: what a terminal put in a bracket to mean yes.
    tick: '<rect x="3" y="3" width="8" height="8" fill="currentColor" ' +
      'shape-rendering="crispEdges"/>',
    up: '<path d="M4 1.5 6.5 4.5h-5z" fill="currentColor" shape-rendering="crispEdges"/>',
    down: '<path d="M4 4.5 1.5 1.5h5z" fill="currentColor" shape-rendering="crispEdges"/>',
  },
});
