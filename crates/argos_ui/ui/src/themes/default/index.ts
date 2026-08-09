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
  description: 'Translucent panes over a near-black backdrop, lit by a single accent.',
  scheme: 'dark',
  tokens: {
    '--backdrop': '#07080a',
    // A wide, very dim pool of light behind the window, so the translucent
    // panes have something to be translucent against.
    '--backdrop-glow':
      'radial-gradient(120% 90% at 50% 0%, rgba(58, 88, 140, 0.16), transparent 62%), ' +
      'radial-gradient(90% 70% at 50% 108%, rgba(40, 62, 104, 0.12), transparent 60%)',

    '--pane': 'rgba(255, 255, 255, 0.045)',
    '--pane-blur': 'blur(24px) saturate(140%)',
    '--pane-border': 'rgba(255, 255, 255, 0.09)',
    '--pane-shadow':
      '0 1px 0 0 rgba(255, 255, 255, 0.06) inset, 0 1.125rem 3rem -1.5rem rgba(0, 0, 0, 0.9)',
    '--pane-radius': '0.75rem',
    '--scrim': 'rgba(4, 5, 7, 0.62)',

    '--row': 'transparent',
    '--row-hover': 'rgba(255, 255, 255, 0.045)',
    '--row-selected': 'rgba(47, 124, 246, 0.16)',
    '--row-selected-border': 'rgba(96, 160, 255, 0.55)',

    '--inset': 'rgba(0, 0, 0, 0.28)',
    '--inset-border': 'rgba(255, 255, 255, 0.08)',
    '--track': 'rgba(255, 255, 255, 0.07)',
    '--ring': '#2f7cf6',
    '--ring-glow': 'rgba(47, 124, 246, 0.42)',

    '--text': '#eef1f5',
    '--text-dim': '#a7b0bd',
    '--text-faint': '#6d7784',

    '--accent': '#2f7cf6',
    '--accent-strong': '#4d94ff',
    '--accent-text': '#ffffff',
    '--accent-glow': 'rgba(47, 124, 246, 0.42)',
    '--action': 'linear-gradient(180deg, #4d94ff, #2f7cf6)',
    '--action-text': '#ffffff',
    '--ok': '#3fb950',
    '--warn': '#d9a441',
    '--danger': '#e5534b',

    '--radius': '0.5rem',
    '--font':
      'system-ui, -apple-system, "Segoe UI Variable Text", "Segoe UI", Roboto, sans-serif',
    '--font-mono': 'ui-monospace, "Cascadia Mono", "JetBrains Mono", Menlo, monospace',
  },
});
