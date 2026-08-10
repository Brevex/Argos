import { theme } from '../contract';

/**
 * The theme the window opens with.
 *
 * Panes are translucent sheets over a near-black backdrop, edge-lit by a
 * one-pixel border a shade brighter than the fill and blurred behind so what
 * is underneath shows through as light rather than as detail. Colour is spent
 * almost entirely on one accent: the selected drive, the scan ring and the
 * button. Everything else is greyscale, so that on a screen full of chrome the
 * saturated things are the ones that mean something.
 */
export default theme({
  id: 'default',
  name: 'Default',
  scheme: 'dark',
  tokens: {
    '--backdrop': '#14171d',
    // A wide, very dim pool of light behind the window, so the translucent
    // panes have something to be translucent against.
    '--backdrop-glow':
      'radial-gradient(120% 90% at 50% 0%, rgba(58, 88, 140, 0.16), transparent 62%), ' +
      'radial-gradient(90% 70% at 50% 108%, rgba(40, 62, 104, 0.12), transparent 60%)',
    '--window-border': 'rgba(202, 208, 217, 0.42)',
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

    '--winbtn-group': 'transparent',
    '--winbtn-group-border': 'transparent',
    '--winbtn-group-radius': '0',
    '--winbtn-divider': 'transparent',
    '--winbtn-text': '#a7b0bd',
    '--winbtn-hover': 'rgba(255, 255, 255, 0.045)',
    '--winbtn-close-hover': '#e5534b',
    '--winbtn-close-hover-text': '#ffffff',
    '--winbtn-stroke': '1.1',

    '--pane': 'rgba(255, 255, 255, 0.045)',
    '--pane-blur': 'blur(24px) saturate(140%)',
    '--pane-border': 'rgba(255, 255, 255, 0.09)',
    '--pane-shadow':
      '0 1px 0 0 rgba(255, 255, 255, 0.06) inset, 0 1.125rem 3rem -1.5rem rgba(0, 0, 0, 0.9)',
    '--pane-radius': '0.75rem',
    '--form-pane': 'rgba(24, 28, 34, 0.88)',
    '--form-pane-border': 'rgba(205, 211, 220, 0.19)',
    '--form-pane-shadow': 'none',
    '--scrim': 'rgba(4, 5, 7, 0.62)',

    '--row': 'transparent',
    '--row-hover': 'rgba(255, 255, 255, 0.045)',
    '--row-selected': 'rgba(47, 124, 246, 0.16)',
    '--row-selected-border': 'rgba(96, 160, 255, 0.55)',
    '--dot-idle': '#6d7784',

    '--inset': 'rgba(0, 0, 0, 0.28)',
    '--inset-border': 'rgba(255, 255, 255, 0.08)',
    '--track': 'rgba(255, 255, 255, 0.07)',
    '--track-edge': 'rgba(255, 255, 255, 0.12)',

    '--ring': '#2f7cf6',
    '--ring-highlight': '#77adff',
    '--ring-shadow': '#1b55ad',
    '--ring-glow': 'rgba(47, 124, 246, 0.42)',
    '--ring-cap': 'round',
    '--ring-pulse': 'transparent',
    '--ring-pulse-duration': '0s',


    '--scanlines': 'none',
    '--text-glow': 'none',

    '--text': '#eef1f5',
    '--text-dim': '#a7b0bd',
    '--text-faint': '#6d7784',

    '--accent': '#2f7cf6',
    '--accent-strong': '#4d94ff',
    '--accent-text': '#ffffff',
    '--accent-glow': 'rgba(47, 124, 246, 0.42)',

    '--action': 'linear-gradient(180deg, #4d94ff, #2f7cf6)',
    '--action-text': '#ffffff',
    '--action-border': 'rgba(96, 160, 255, 0.55)',
    '--action-shadow': '0 0.375rem 1.125rem -0.5rem rgba(47, 124, 246, 0.42)',

    '--ok': '#3fb950',
    '--warn': '#d9a441',
    '--danger': '#e5534b',

    '--radius': '0.5rem',
    '--font':
      'system-ui, -apple-system, "Segoe UI Variable Text", "Segoe UI", Roboto, sans-serif',
    '--font-mono': 'ui-monospace, "Cascadia Mono", "JetBrains Mono", Menlo, monospace',
  },
  icons: {
    // A drive drawn as an outline: a case, a platter, a spindle and the arm
    // over it. Greyscale but for the accent the layout puts on the selected
    // row, which is why every stroke here takes its colour from the caller.
    disk:
      '<rect x="0.7" y="0.7" width="18.6" height="14.6" rx="2" fill="none" ' +
      'stroke="currentColor" stroke-width="1.05"/>' +
      '<circle cx="8.2" cy="8" r="5" fill="none" stroke="currentColor" stroke-width="0.95"/>' +
      '<circle cx="8.2" cy="8" r="1.15" fill="currentColor"/>' +
      '<path d="M16.4 4.1 12.1 9.7" fill="none" stroke="currentColor" stroke-width="1.05" ' +
      'stroke-linecap="round"/>' +
      '<circle cx="16.6" cy="3.6" r="1.05" fill="currentColor"/>',
    minimise: '<path d="M2 6h8" fill="none" stroke="currentColor" stroke-linecap="round"/>',
    maximise: '<rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor"/>',
    close: '<path d="M3 3l6 6M9 3l-6 6" fill="none" stroke="currentColor" stroke-linecap="round"/>',
  },
});
