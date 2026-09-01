import { theme } from '../contract';

/**
 * Aero: the glass desktop.
 */
export default theme({
  id: 'aero',
  name: 'Aero',
  scheme: 'light',
  // A tick box and a bevelled radio: the source draws both, and a sliding
  // switch is a decade too late for this window.
  controls: { checkbox: 'checkbox', choice: 'radio' },
  tokens: {
    // frame
    // `.window`: a translucent blue wash, a white band low on the glass, a
    // black outline and a white line just inside it.
    '--backdrop': 'rgba(70, 162, 255, 0.37)',
    '--backdrop-glow':
      'linear-gradient(180deg, rgba(231, 240, 255, 0) 10%, #ffffff 35%, ' +
      '#ffffff 35%, rgba(253, 254, 255, 0) 36%, rgba(231, 240, 255, 0) 100%)',
    // The source has no grain. Neither does this.
    '--backdrop-noise': 'none',

    // `outline: solid 1px #000000` and `inset 0 0 0 1px #fcfcfc`.
    '--window-edge': '#000000',
    '--window-border': '#fcfcfc',
    '--window-radius': '0.33rem',
    '--window-shadow': '0 3px 10px #000000',
    // `padding: 5px` with `padding-top: 0` — the title bar meets the top edge
    // and the body sits inside a five-pixel band on the other three sides.
    '--window-inset': '0.33rem',

    // `.window-body`: flat, opaque, cut into the frame.
    '--main-surface': '#f0eff2',
    // The source writes `outline: inset 1px`, which is a border style rather
    // than a colour; this is the grey that style resolves to on that surface.
    '--main-border': '#9d9d9d',
    '--main-shadow': 'none',
    '--main-radius': '0',

    '--titlebar': 'transparent',
    '--titlebar-border': 'transparent',
    '--titlebar-text': '#000000',
    // `.title-bar-text`: eight stacked white glows. That stack is what makes
    // black text legible on glass, and one glow does not do it.
    '--titlebar-text-size': '0.94rem',
    '--titlebar-text-weight': '400',
    '--titlebar-text-shadow':
      '0 0 10px #fff, 0 0 10px #fff, 0 0 10px #fff, 0 0 10px #fff, ' +
      '0 0 10px #fff, 0 0 10px #fff, 0 0 10px #fff, 0 0 10px #fff',

    // caption group
    // `.title-bar-controls`: pulled up so the buttons meet the window's top
    // edge, 20px tall, 27px wide, and 45px for the close.
    '--winbtn-align': 'flex-start',
    '--winbtn-offset-top': '0rem',
    '--winbtn-offset-right': '0.33rem',
    '--winbtn-height': '1.38rem',
    '--winbtn-width': '1.86rem',
    '--winbtn-close-width': '3.1rem',
    '--winbtn-glyph': '0.97rem',
    '--winbtn-border': 'transparent',
    '--winbtn-radius': '0',
    '--winbtn-gap': '0rem',
    '--winbtn-group-pull': '0rem',
    '--winbtn-group': 'transparent',
    '--winbtn-group-border': 'transparent',
    // Square where the group meets the top edge, 4px where it leaves it.
    '--winbtn-group-radius': '0 0 0.27rem 0.27rem',
    '--winbtn-group-shadow': 'none',
    // `.title-bar-controls > button:hover { outline: solid 1px #000 }` is the
    // only edge the source draws between them.
    '--winbtn-divider': 'rgba(0, 0, 0, 0.35)',
    '--winbtn-divider-light': 'rgba(255, 255, 255, 0.35)',

    // Minimise and maximise, and what they do under the pointer and under the
    // press. The cyan at the foot of the hover fill is the source's, and it is
    // the whole reason an Aero caption button reads as lit rather than tinted.
    '--winbtn-face':
      'linear-gradient(180deg, #b8c5d3 0%, rgba(174, 174, 174, 0.494) 50%, ' +
      'rgba(132, 132, 132, 0.475) 50%, #728aa9 100%)',
    '--winbtn-text': '#000000',
    '--winbtn-hover-text': '#000000',
    '--winbtn-hover':
      'linear-gradient(180deg, #b3d6ed 0%, #83b5d7 50%, #2978ae 50%, #09fdfa 100%)',
    '--winbtn-hover-filter': 'drop-shadow(0 4px 4px #5cc4ef)',
    '--winbtn-active':
      'linear-gradient(180deg, #a0bacb 0%, #57a4b3 50%, #0b3f5b 50%, #13d5c4 100%)',

    // Close. Red at rest, and the hover carries a band of yellow at its foot —
    // which is the source's, and is what that button actually did.
    '--winbtn-close':
      'linear-gradient(180deg, #efb1a1 0%, #d5836f 51%, #ab4732 51%, #c17f6e 100%)',
    '--winbtn-close-text': '#ffffff',
    '--winbtn-close-hover':
      'linear-gradient(180deg, #f99b8b 0%, #ed6c56 51%, #ab220c 51%, ' +
      '#d22405 72%, #f4e679 100%)',
    '--winbtn-close-hover-text': '#ffffff',
    '--winbtn-close-hover-filter': 'drop-shadow(0 4px 4px rgba(255, 0, 0, 0.467))',
    '--winbtn-close-active':
      'linear-gradient(180deg, #d1a993 0%, #b7745b 51%, #8a1f09 52%, #f0c928 100%)',
    '--winbtn-stroke': '1',

    // The source has no unfocused window. Withdrawing the faces and the red is
    // what that desktop did, and it is the one place here that is inference
    // rather than port.
    '--winbtn-face-off': 'rgba(255, 255, 255, 0.25)',
    '--winbtn-close-off': 'rgba(255, 255, 255, 0.25)',
    '--winbtn-group-border-off': 'transparent',
    '--winbtn-divider-off': 'rgba(0, 0, 0, 0.18)',

    // panes
    // `fieldset`: a hairline in `#cdd7db` with a white line inside it.
    '--pane': '#f0eff2',
    '--pane-blur': 'none',
    '--pane-border': '#cdd7db',
    '--pane-shadow': 'inset 0 0 0 1px #ffffff',
    '--pane-radius': '0.2rem',
    '--form-pane': 'transparent',
    '--form-pane-border': 'transparent',
    '--form-pane-shadow': 'none',
    // A dialog is a window: the same glass, the same band around the panel,
    // the same height, the same caption group.
    '--dialog-strip':
      'linear-gradient(180deg, rgba(231, 240, 255, 0) 10%, #ffffff 35%, ' +
      '#ffffff 35%, rgba(253, 254, 255, 0) 36%, rgba(231, 240, 255, 0) 100%), ' +
      'rgba(70, 162, 255, 0.37)',
    '--dialog-strip-height': '2.9rem',
    '--dialog-inset': '0.33rem',
    '--dialog-blur': 'blur(3px)',
    // Deep enough that the glass over it reads in the same register as the
    // glass over the desktop: this material takes the colour of whatever is
    // behind it, and behind a dialog is a light window.
    '--scrim': 'rgba(12, 32, 56, 0.62)',

    // rows
    // Not in the source, which has no list. Built from its own hover blues.
    '--row': 'transparent',
    '--row-hover': 'linear-gradient(180deg, #f3f9fe 0%, #eaf6fd 51%, #d9f0fc 51%, #cbe8fb 100%)',
    '--row-selected': 'linear-gradient(180deg, #eaf6fd 0%, #d9f0fc 51%, #bce5fc 51%, #a7d9f5 100%)',
    '--row-selected-border': '#3c7fb1',
    '--row-selected-shadow': 'inset 0 0 0 1px #ffffff',
    '--row-radius': '0.2rem',

    // `input[type="radio"]`: a sunken white well with a cyan bead in a dark rim.
    '--choice': '#f6f6f6',
    '--choice-border': '#8e8f8f',
    '--choice-border-selected': '#8e8f8f',
    '--choice-mark':
      'radial-gradient(circle at 50% 50%, #7cd3eb 0 0.13rem, #1d91d0 0.13rem 0.17rem, ' +
      '#2e648b 0.17rem 0.23rem, transparent 0.23rem), #f6f6f6',
    '--choice-shadow':
      'inset 0 0 0 1.5px #f4f4f4, inset 1px 1px 0 1.5px #aeaeae, ' +
      'inset -1px 0 0 1.5px #dddddd, inset 3px 3px 6px #cccccc',

    // fields
    // A list box and a text field on this desktop are white, cut into the
    // surface with the fieldset's hairline and its white inner line.
    '--inset': '#ffffff',
    '--inset-border': '#cdd7db',
    '--inset-border-hover': '#3c7fb1',
    '--inset-shadow': 'inset 0 0 0 1px #ffffff, inset 0 1px 3px rgba(0, 0, 0, 0.12)',
    '--input-radius': '0.2rem',

    // The progress arc's channel. Not in the source; the greys are the ones it
    // builds its resting button out of.
    '--track': '#dbdbdb',
    '--track-edge': '#8e8f8f',
    '--track-lit': '#fcfcfc',
    '--track-shadow': 'inset 0 1px 3px rgba(0, 0, 0, 0.16)',

    // buttons
    // `button`, verbatim: a four-stop gradient with the break at the middle,
    // a grey outline and a white line inside it. Hover turns it blue; the
    // press darkens it and lays a two-pixel lit edge inside.
    '--button':
      'linear-gradient(180deg, #fcfcfc 0%, #ebebeb 51%, #dbdbdb 51%, #cfcfcf 100%)',
    '--button-hover':
      'linear-gradient(180deg, #eaf6fd 0%, #d9f0fc 51%, #bce5fc 51%, #a7d9f5 100%)',
    '--button-active':
      'linear-gradient(180deg, #e5f4fc 0%, #c4e5f6 51%, #98d1ef 51%, #68b2da 100%)',
    '--button-text': '#000000',
    '--button-text-hover': '#000000',
    '--button-border': '#757575',
    '--button-border-hover': '#3c7fb1',
    '--button-shadow': 'inset 0 0 0 1px #fcfcfc',
    '--button-shadow-active': 'inset 0 0 0 2px #86c6e8',
    '--button-radius': '0.2rem',

    // The one button that starts and stops a scan. The source has a single
    // button and no notion of a default one, so this is that button with the
    // glow the system put around the one the Enter key would press.
    '--action':
      'linear-gradient(180deg, #fcfcfc 0%, #ebebeb 51%, #dbdbdb 51%, #cfcfcf 100%)',
    '--action-hover':
      'linear-gradient(180deg, #eaf6fd 0%, #d9f0fc 51%, #bce5fc 51%, #a7d9f5 100%)',
    '--action-active':
      'linear-gradient(180deg, #e5f4fc 0%, #c4e5f6 51%, #98d1ef 51%, #68b2da 100%)',
    '--action-text': '#000000',
    '--action-text-hover': '#000000',
    '--action-border': '#3c7fb1',
    '--action-shadow': 'inset 0 0 0 1px #fcfcfc, 0 0 0.34rem rgba(60, 127, 177, 0.85)',
    '--action-shadow-active': 'inset 0 0 0 2px #86c6e8',

    '--link': '#0066cc',
    '--link-hover': '#3c7fb1',

    '--disabled-text': '#8d8d8d',
    '--disabled-opacity': '1',

    '--focus-outline': '1px dotted #000000',
    '--focus-offset': '-3px',

    // progress
    // Not in the source. The green is the one Windows 7's own progress bar
    // used, and the bands either side of it are mixed from it.
    '--ring': '#00d328',
    '--ring-highlight': 'color-mix(in srgb, var(--ring) 22%, #ffffff)',
    '--ring-edge': 'color-mix(in srgb, var(--ring) 62%, #ffffff)',
    '--ring-glow': 'transparent',
    '--ring-cap': 'butt',
    '--progress-running': '#00d328',
    '--progress-paused': '#e0a21b',
    '--progress-done': '#00d328',
    '--progress-cancelled': '#9d9d9d',
    '--progress-failed': '#ab4732',
    '--sheen': 'rgba(255, 255, 255, 0.5)',
    '--sheen-duration': '2.1s',
    '--sheen-easing': 'cubic-bezier(0.45, 0, 0.55, 1)',

    // controls
    // The switch is not this theme's idiom, but the contract is total, so it
    // is drawn in the same greys and blues as everything else.
    '--switch-track': 'linear-gradient(180deg, #cfcfcf 0%, #ebebeb 100%)',
    '--switch-track-on': 'linear-gradient(180deg, #68b2da 0%, #a7d9f5 100%)',
    '--switch-border': '#757575',
    '--switch-border-on': '#3c7fb1',
    '--switch-thumb': 'linear-gradient(180deg, #fcfcfc 0%, #dbdbdb 100%)',
    '--switch-radius': '0.2rem',
    '--switch-thumb-radius': '0.14rem',

    // `input[type="checkbox"] + label::before`, verbatim — the well, its four
    // inset lines and the tick's own colour.
    '--check-box': '#f6f6f6',
    '--check-box-checked': '#f6f6f6',
    '--check-border': '#8e8f8f',
    '--check-border-checked': '#8e8f8f',
    '--check-border-hover': '#3c7fb1',
    '--check-mark': '#4e64a1',
    '--check-radius': '0',
    '--check-shadow':
      'inset 0 0 0 1px #f4f4f4, inset 1px 1px 0 1px #aeaeae, ' +
      'inset -1px -1px 0 1px #dddddd, inset 3px 3px 6px #cccccc',
    '--check-shadow-hover':
      'inset 0 0 0 1px #def9fa, inset 1px 1px 0 1px #79c6f9, ' +
      'inset -1px -1px 0 1px #c6e9fc, inset 3px 3px 6px #b1dffd',

    // The seal on a drive row. Not in the source; the amber is this
    // application's warning colour, given the fieldset's edge treatment.
    '--badge': 'linear-gradient(180deg, #fff8e1 0%, #ffe9b0 100%)',
    '--badge-border': '#d9a441',
    '--badge-text': '#7a5210',
    '--badge-quiet': 'linear-gradient(180deg, #fcfcfc 0%, #ebebeb 100%)',
    '--badge-quiet-border': '#cdd7db',
    '--badge-quiet-text': '#5a5a5a',
    '--badge-radius': '0.14rem',

    // Not in the source. The thumb is its button, the channel its sunken well.
    '--scrollbar-track': '#f0eff2',
    '--scrollbar-thumb':
      'linear-gradient(90deg, #fcfcfc 0%, #ebebeb 51%, #dbdbdb 51%, #cfcfcf 100%)',
    '--scrollbar-thumb-hover':
      'linear-gradient(90deg, #eaf6fd 0%, #d9f0fc 51%, #bce5fc 51%, #a7d9f5 100%)',
    '--scrollbar-border': '#cdd7db',
    '--scrollbar-radius': '0.14rem',

    // ground
    '--scanlines': 'none',
    '--text-glow': 'none',
    '--bevel-raised': 'inset 0 0 0 1px #fcfcfc',
    '--bevel-sunken': 'inset 0 0 0 1px #ffffff, inset 0 1px 3px rgba(0, 0, 0, 0.12)',
    // The break the source puts down the middle of everything clickable.
    '--specular':
      'linear-gradient(180deg, rgba(255, 255, 255, 0.5) 0%, ' +
      'rgba(255, 255, 255, 0.12) 51%, rgba(255, 255, 255, 0) 51%)',

    '--text': '#000000',
    '--text-dim': '#3a3a3a',
    '--text-faint': '#767676',
    '--heading': '#000000',

    '--accent': '#3c7fb1',
    '--accent-strong': '#2c628b',
    '--accent-text': '#ffffff',
    '--ok': '#1a7a2e',
    '--warn': '#8a5b00',
    '--danger': '#ab4732',

    // The source loads Segoe UI from Microsoft's own CDN. This window ships
    // Selawik instead — metric-compatible, bundled, and it renders offline.
    '--font': '"Segoe UI", Selawik, "Noto Sans", Carlito, sans-serif',
  },
  icons: {
    // The three caption glyphs, from the source's own `assets/*.svg`: a shape
    // filled with a white-to-grey gradient over a `#535666` outline. Placed
    // into the twelve-unit box this layout draws them in, and their gradient
    // ids made unique so three of them on one page do not share one.
    minimise:
      '<defs><linearGradient id="aero-min-f" x1="0" y1="1" x2="0" y2="0">' +
      '<stop offset="0" stop-color="#e4e4e4"/><stop offset="1" stop-color="#fff"/>' +
      '</linearGradient></defs>' +
      '<g transform="translate(1.485 4.122)">' +
      '<rect x="0.75" y="0.75" width="7.52941" height="2.25686" fill="url(#aero-min-f)"/>' +
      '<path d="M8.27942.75V3.00687H.75V.75H8.27942m.75-.75H0V3.75687H9.02942V0Z" fill="#535666"/>' +
      '</g>',
    maximise:
      '<defs><linearGradient id="aero-max-f" x1="0" y1="1" x2="0" y2="0">' +
      '<stop offset="0" stop-color="#dadada"/><stop offset="1" stop-color="#fff"/>' +
      '</linearGradient></defs>' +
      '<g transform="translate(1.479 2.229)">' +
      '<path d="M.75.75V6.79163H8.2917V.75ZM6.78474,5.26385H2.257V2.20831H6.78474Z" ' +
      'fill="url(#aero-max-f)"/>' +
      '<path d="M8.29169.75V6.79163H.75V.75H8.29169M2.257,5.26385H6.78473V2.20831H2.257V5.26385' +
      'M9.04169,0H0V7.54163H9.04169V0ZM3.007,2.95831H6.03473V4.51385H3.007V2.95831Z" ' +
      'fill="#535666"/>' +
      '</g>',
    close:
      '<defs><linearGradient id="aero-close-f" x1="0" y1="1" x2="0" y2="0">' +
      '<stop offset="0" stop-color="#dadada"/><stop offset="1" stop-color="#fff"/>' +
      '</linearGradient></defs>' +
      '<g transform="translate(1.158 2.287)">' +
      '<polygon points="5.949 3.71 8.18 6.676 5.949 6.676 4.848 5.186 4.842 5.194 4.836 5.186 ' +
      '3.735 6.676 1.504 6.676 3.735 3.71 1.504 0.75 3.71 0.75 4.842 2.223 5.975 0.75 8.18 0.75 ' +
      '5.949 3.71" fill="url(#aero-close-f)"/>' +
      '<path d="M8.18013.75,5.94894,3.70973l2.23119,2.9662H5.94894L4.84834,5.18562l-.00617.00824' +
      'L4.8363,5.18562,3.73539,6.67593H1.5045l2.23089-2.9662L1.5045.75h2.205L4.84217,2.22266,' +
      '5.97512.75h2.205M9.68475,0H5.60585L5.38068.29268,4.84225.99255,4.304.29276,4.07884,0H0' +
      'L.90559,1.20143l1.891,2.50879L.90511,6.22513.002,7.42593H4.1138l.22485-.30436.50359-.68172' +
      '.50339.68164.22484.30444H9.68279L8.7795,6.22508,6.88781,3.71022,8.779,1.20146,9.68475,0Z" ' +
      'fill="#545767"/>' +
      '</g>',
    // The tick the source puts in a checked box, and the two arrows a number
    // field steps with — drawn in its own ink.
    tick:
      '<path d="M2 7.4l3.4 3.4L12 3.6" fill="none" stroke="currentColor" stroke-width="2.2" ' +
      'stroke-linecap="square"/>',
    up: '<path d="M4 1.4l3 3.2H1z" fill="currentColor"/>',
    down: '<path d="M4 4.6l-3-3.2h6z" fill="currentColor"/>',
  },
});
