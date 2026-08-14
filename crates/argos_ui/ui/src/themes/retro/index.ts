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
 * Two things are allowed to break that flatness, and only two. The progress
 * arcs glow, the way a phosphor dot actually did. And a grid of unlit lines
 * lies over the whole window, so every surface — the arcs included — reads as
 * something drawn on cells rather than on pixels.
 */
export default theme({
  id: 'retro',
  name: 'Retro',
  scheme: 'dark',
  tokens: {
    '--backdrop': '#030b05',
    '--backdrop-glow': 'none',
    '--window-border': '#3ca84c',
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
    // Half the difference between the bar and a button, so the three glyphs

    // No strip and no dividers: three bare glyphs, which is all a terminal
    // would have drawn.
    '--winbtn-group': 'transparent',
    '--winbtn-group-border': 'transparent',
    '--winbtn-group-radius': '0',
    '--winbtn-divider': 'transparent',
    '--winbtn-text': '#6dff6d',
    '--winbtn-hover': '#0d2c14',
    '--winbtn-close-hover': '#0d2c14',
    '--winbtn-close-hover-text': '#8dff8d',
    '--winbtn-stroke': '1.1',

    // Opaque, unblurred, unshadowed. A pane is the same ground as the window
    // with a rule drawn around it.
    '--pane': '#030b05',
    '--pane-blur': 'none',
    '--pane-border': '#3ca84c',
    '--pane-shadow': 'none',
    '--pane-radius': '0',
    '--form-pane': 'transparent',
    '--form-pane-border': '#3ca84c',
    '--form-pane-shadow': 'none',
    '--scrim': '#030b05',

    '--row': 'transparent',
    '--row-hover': '#0b2211',
    '--row-selected': '#0f3018',
    '--row-selected-border': '#4ee44e',
    '--dot-idle': 'transparent',

    '--inset': '#020803',
    '--inset-border': '#3ca84c',
    '--track': '#24682f',
    '--track-edge': '#153c1c',

    '--ring': '#68f068',
    '--ring-highlight': '#68f068',
    '--ring-shadow': '#68f068',
    '--ring-glow': 'rgba(104, 240, 104, 0.55)',
    // Flat ends. A rounded cap is a shape a cell grid cannot produce.
    '--ring-cap': 'butt',
    '--ring-pulse': 'transparent',
    '--ring-pulse-duration': '0s',

    // The cell grid: unlit lines every third pixel, both ways, over
    // everything. Fine enough to read as texture rather than as a pattern.

    // Rows, and only rows. The cross-hatch of a grid is what made the last
    // attempt read as a glitch: a display drew in lines, so the texture is
    // lines — one unlit row in every two, at the smallest pitch that survives
    // being drawn. It goes on lit surfaces and on nothing else, so the frame,
    // the title and the window edge stay as sharp as they were.
    '--scanlines':
      'repeating-linear-gradient(180deg, rgba(0, 0, 0, 0.32) 0 1px, transparent 1px 2px)',
    // The bloom around a lit phosphor dot. This, rather than any grid, is what
    // makes green text read as a screen instead of as green text.
    '--text-glow': '0 0 0.42em rgba(104, 240, 104, 0.5)',

    '--text': '#5cf85c',
    '--text-dim': '#43cc43',
    '--text-faint': '#309e30',

    '--accent': '#3ad13a',
    '--accent-strong': '#6dff6d',
    '--accent-text': '#02150a',

    // Outlined, not filled: the border and the label carry it, the way a
    // terminal drew a control it could not shade.
    '--action': 'transparent',
    '--action-text': '#6dff6d',
    '--action-border': '#4ee44e',
    '--action-shadow': 'none',

    '--ok': '#4ee44e',
    '--warn': '#c8d94a',
    '--danger': '#ff5a5a',

    // Square, like everything else. A phosphor block that slides.
    '--switch-track': '#020803',
    '--switch-track-on': '#0b3a12',
    '--switch-border': '#3ca84c',
    '--switch-border-on': '#6dff6d',
    '--switch-thumb': '#6dff6d',
    '--switch-radius': '0',
    '--switch-thumb-radius': '0',
    '--radius': '0',
    '--font': 'ui-monospace, "Cascadia Mono", "JetBrains Mono", Consolas, Menlo, monospace',
  },
  icons: {
    // Line art on the character grid: a case, a slot and two spindles. Every
    // stroke is one unit wide and every corner is square, because a display
    // that drew cells could not draw anything else.
    disk:
      '<rect x="1" y="3.5" width="18" height="9" fill="none" stroke="currentColor" ' +
      'stroke-width="1"/>' +
      '<rect x="2.8" y="5.4" width="5.4" height="5.2" fill="none" stroke="currentColor" ' +
      'stroke-width="1"/>' +
      '<path d="M10.4 6.4h6.4M10.4 8h6.4" fill="none" stroke="currentColor" stroke-width="1"/>' +
      '<rect x="10.4" y="9.6" width="1.6" height="1.4" fill="currentColor"/>' +
      '<rect x="13" y="9.6" width="1.6" height="1.4" fill="currentColor"/>',
    minimise: '<path d="M2 8h8" fill="none" stroke="currentColor" stroke-linecap="square"/>',
    maximise: '<rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor"/>',
    close: '<path d="M2.6 2.6l6.8 6.8M9.4 2.6l-6.8 6.8" fill="none" stroke="currentColor" ' +
      'stroke-linecap="square"/>',
  },
});
