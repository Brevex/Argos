import { theme } from '../contract';

/**
 * Aero: Frutiger Aero — glass, water-light and sky.
 *
 * The look is optimistic and wet: everything is a surface with light coming
 * through it rather than a shape with a colour. A translucent frame surrounds
 * the window on all four sides and the title bar is part of that frame rather
 * than a band laid across the top of the content — there is no rule between
 * them, and the same glass runs from the title down the left edge, under the
 * client area and up the right. The specular highlight sits at the top of the
 * glass, where the light source is, and fades before the middle.
 *
 * Inside it is one near-white sheet with a thin cornflower rule, and the
 * controls sit on it as separate objects: the drive table and the destination
 * field share no panel, because they are two questions rather than one.
 *
 * Blue is the colour of choice — the selected drive, the button. Green is the
 * colour of work under way, and the highlight sweeping the filled arc is the
 * one animation here, because that sweep is what a progress bar of this
 * moment did.
 */
export default theme({
  id: 'aero',
  name: 'Aero',
  scheme: 'light',
  tokens: {
    // The frame band. Glass: a tinted, translucent surface with the light
    // source above it, so the specular is strongest at the very top and gone
    // by the middle. Darker and bluer than the sheet it surrounds, because a
    // frame reads as glass only when what is behind it is dimmer than what is
    // on it.
    '--backdrop': 'rgba(171, 204, 232, 0.68)',
    '--backdrop-glow':
      // The specular on the very top edge, where the light is. Bright, and
      // over before the title has finished.
      'linear-gradient(180deg, rgba(255, 255, 255, 0.96) 0%, rgba(255, 255, 255, 0.54) 5%, ' +
      'rgba(255, 255, 255, 0.18) 16%, rgba(255, 255, 255, 0.04) 38%, ' +
      'rgba(255, 255, 255, 0) 60%), ' +
      // The gleam: light lying across the upper left of the pane, the way it
      // lies on a sheet of glass rather than on a painted surface.
      'radial-gradient(115% 70% at 20% -14%, rgba(255, 255, 255, 0.6), transparent 56%), ' +
      // Light wrapping the bottom edge as it passes through.
      'linear-gradient(0deg, rgba(255, 255, 255, 0.32) 0%, transparent 24%), ' +
      // The body of the glass, cooler where it is thickest.
      'linear-gradient(180deg, rgba(219, 237, 251, 0.5) 0%, rgba(160, 200, 232, 0.56) 100%)',

    // A white inner edge, a blue-grey outer line to define it against the
    // desktop, and a wide soft shadow underneath.
    '--window-border': 'rgba(255, 255, 255, 0.86)',
    '--window-radius': '0.62rem',
    '--window-shadow':
      '0 0 0 1px rgba(70, 121, 165, 0.55), 0 0.55rem 0.4rem -0.3rem rgba(28, 66, 100, 0.22), ' +
      '0 1.5rem 2.4rem -0.7rem rgba(30, 69, 103, 0.5)',
    '--window-inset': '0.44rem',

    // The client sheet: near-white, faintly blue, lit from the middle.
    '--main-surface':
      'radial-gradient(80% 62% at 50% 40%, rgba(255, 255, 255, 0.98), rgba(255, 255, 255, 0) 72%), ' +
      'linear-gradient(180deg, #fdfeff 0%, #f2f8fe 46%, #e7f2fd 100%)',
    '--main-border': '#93bfe2',
    '--main-shadow':
      '0 0 0 1px rgba(255, 255, 255, 0.9), 0 1px 0 rgba(255, 255, 255, 0.95) inset',
    '--main-radius': '0.42rem',

    // No fill and no rule: the title bar is the frame, continued.
    '--titlebar': 'transparent',
    '--titlebar-border': 'transparent',
    '--titlebar-text': '#1b3f5f',

    // One glass strip in the corner, divided into three by hairlines, rounded
    // on its bottom corners only — it is flush with the top of the frame.
    '--winbtn-group':
      'linear-gradient(180deg, rgba(255, 255, 255, 0.94) 0%, rgba(240, 249, 255, 0.86) 48%, ' +
      'rgba(206, 231, 250, 0.86) 52%, rgba(226, 242, 253, 0.9) 100%)',
    '--winbtn-group-border': 'rgba(122, 168, 207, 0.85)',
    '--winbtn-group-radius': '0.28rem',
    '--winbtn-divider': 'rgba(122, 168, 207, 0.5)',
    '--winbtn-text': '#1b3f5f',
    '--winbtn-hover':
      'linear-gradient(180deg, rgba(255, 255, 255, 1) 0%, rgba(226, 243, 255, 0.95) 100%)',
    '--winbtn-close-hover': 'linear-gradient(180deg, #ef8074 0%, #e05043 48%, #c9291d 52%, #d8443a 100%)',
    '--winbtn-close-hover-text': '#ffffff',
    '--winbtn-stroke': '1.35',

    // Panes that remain panes: the statistics strip and the theme dialog.
    '--pane': 'linear-gradient(180deg, rgba(255, 255, 255, 0.96), rgba(246, 251, 255, 0.94))',
    '--pane-blur': 'none',
    '--pane-border': '#a6c8e6',
    '--pane-shadow': '0 1px 0 0 rgba(255, 255, 255, 0.95) inset',
    '--pane-radius': '0.3rem',
    // The drive table and the destination field are two objects on the sheet,
    // not two halves of a third one.
    '--form-pane': 'transparent',
    '--form-pane-border': 'transparent',
    '--form-pane-shadow': 'none',
    '--scrim': 'rgba(186, 218, 243, 0.62)',

    '--row': 'transparent',
    '--row-hover': 'linear-gradient(180deg, rgba(238, 248, 255, 0.95), rgba(219, 238, 252, 0.9))',
    '--row-selected': 'linear-gradient(180deg, #eaf5fe 0%, #d9ecfc 52%, #cbe4fa 100%)',
    '--row-selected-border': '#6ba5da',
    // Nothing at all on a row that is not selected: the reference shows one
    // mark on the chosen row and clean space on the others.
    '--dot-idle': 'transparent',

    '--inset': 'linear-gradient(180deg, #ffffff 0%, #fbfdff 100%)',
    '--inset-border': '#a6c8e6',
    '--track': '#dde8f2',
    '--track-edge': '#c4d8e9',

    // Green, and not the accent: on a window this blue the arcs are the one
    // thing that says work is under way, and blue on blue would lose them.
    '--ring': '#3fbb28',
    '--ring-highlight': '#8ce860',
    '--ring-shadow': '#1f8f1a',
    '--ring-glow': 'rgba(76, 202, 45, 0.62)',
    '--ring-cap': 'round',
    '--ring-pulse': 'rgba(122, 196, 74, 0.85)',
    '--ring-pulse-duration': '2.6s',


    '--scanlines': 'none',
    '--text-glow': 'none',

    '--text': '#1b3f5f',
    '--text-dim': '#2f6da8',
    '--text-faint': '#7ba0be',

    '--accent': '#1b7fd4',
    '--accent-strong': '#1b7fd4',
    '--accent-text': '#ffffff',
    '--accent-glow': 'rgba(27, 127, 212, 0.45)',

    // The highlight breaks at the middle instead of fading, which is what
    // makes a button of this era read as glass rather than as a gradient.
    '--action':
      'linear-gradient(180deg, #6dbcf2 0%, #4aa1e9 48%, #2a80d2 50%, #1f6ec0 88%, #2b7ac9 100%)',
    '--action-text': '#ffffff',
    '--action-border': '#1a629f',
    '--action-shadow':
      '0 0 0 1px rgba(255, 255, 255, 0.72) inset, 0 0.35rem 0.6rem -0.3rem rgba(15, 70, 122, 0.55)',

    '--ok': '#2f8f2a',
    '--warn': '#a5761b',
    '--danger': '#c0392b',

    '--radius': '0.24rem',
    '--font':
      '"Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system, Roboto, sans-serif',
    '--font-mono': 'ui-monospace, "Cascadia Mono", Consolas, Menlo, monospace',
  },
  icons: {
    // A drive seen face on, the way the system's own drive icon draws one: a
    // silver case with the light catching its top edge, a dark bezel on the
    // left with two vent slots and the green activity lamp, and three ribs
    // across the body. Flat bands rather than gradients, so eight of them on
    // screen cost eight rectangles.
    disk:
      '<rect x="0.6" y="2.4" width="18.8" height="11.2" rx="1.7" fill="#c4d0db"/>' +
      '<rect x="0.6" y="2.4" width="18.8" height="4.6" rx="1.7" fill="#f2f6fa"/>' +
      '<rect x="0.6" y="6.6" width="18.8" height="2.2" fill="#dae2ea"/>' +
      '<rect x="0.6" y="10" width="18.8" height="3.6" fill="#a7b5c2"/>' +
      '<rect x="0.6" y="2.4" width="18.8" height="11.2" rx="1.7" fill="none" ' +
      'stroke="#74838f" stroke-width="0.8"/>' +
      '<rect x="2" y="4.2" width="5.8" height="7.6" rx="0.8" fill="#4e5862"/>' +
      '<rect x="2.8" y="5.3" width="4.2" height="1" rx="0.5" fill="#93a0ab"/>' +
      '<rect x="2.8" y="7" width="4.2" height="1" rx="0.5" fill="#93a0ab"/>' +
      '<circle cx="4.9" cy="10.2" r="0.85" fill="#63e04c"/>' +
      '<rect x="9.4" y="5" width="8.2" height="1.1" rx="0.55" fill="#9aa8b5"/>' +
      '<rect x="9.4" y="7.4" width="8.2" height="1.1" rx="0.55" fill="#adbac6"/>' +
      '<rect x="9.4" y="9.8" width="5.6" height="1.1" rx="0.55" fill="#adbac6"/>',
    minimise: '<path d="M2.5 8.4h7" fill="none" stroke="currentColor" stroke-linecap="square"/>',
    // The maximise glyph of the era: a box with a heavier title edge.
    maximise:
      '<rect x="2.5" y="2.8" width="7" height="6.6" fill="none" stroke="currentColor"/>' +
      '<path d="M2.5 4.4h7" fill="none" stroke="currentColor"/>',
    close: '<path d="M3 3l6 6M9 3l-6 6" fill="none" stroke="currentColor" stroke-linecap="square"/>',
  },
});
