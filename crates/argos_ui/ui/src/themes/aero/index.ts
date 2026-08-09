import { theme } from '../contract';

/**
 * Light glass over a pale blue sky.
 *
 * Sheets of frosted white sit on a wash of daylight, edged with a thin blue
 * line and lit from above. Blue is the colour of choice — the selected drive,
 * the button — and green is the colour of work happening, which is why the
 * progress arcs are the one thing on screen that is not in the blue family.
 * Text is a deep navy rather than black, so a screen this bright is still
 * comfortable to watch for the length of a scan.
 */
export default theme({
  id: 'aero',
  name: 'Aero',
  description: 'Frosted white sheets on a pale blue sky, with green progress arcs.',
  scheme: 'light',
  tokens: {
    '--backdrop': '#dcecf8',
    // Daylight from above and a cooler pool at the bottom edge, so the frosted
    // sheets have something to be frosted against.
    '--backdrop-glow':
      'radial-gradient(120% 90% at 50% -12%, rgba(255, 255, 255, 0.95), transparent 62%), ' +
      'radial-gradient(70% 55% at 14% 6%, rgba(255, 255, 255, 0.7), transparent 58%), ' +
      'radial-gradient(95% 75% at 86% 106%, rgba(146, 191, 230, 0.5), transparent 64%)',

    '--pane': 'rgba(255, 255, 255, 0.6)',
    '--pane-blur': 'blur(20px) saturate(130%)',
    '--pane-border': 'rgba(120, 170, 212, 0.55)',
    '--pane-shadow':
      '0 1px 0 0 rgba(255, 255, 255, 0.85) inset, 0 0.75rem 1.75rem -0.9rem rgba(28, 72, 112, 0.4)',
    '--pane-radius': '0.5rem',
    '--scrim': 'rgba(196, 222, 242, 0.66)',

    '--row': 'transparent',
    '--row-hover': 'rgba(255, 255, 255, 0.6)',
    '--row-selected': 'rgba(151, 200, 240, 0.42)',
    '--row-selected-border': '#5b9bd5',

    '--inset': 'rgba(255, 255, 255, 0.82)',
    '--inset-border': 'rgba(126, 172, 210, 0.55)',
    '--track': 'rgba(164, 198, 226, 0.42)',

    // Green, and not the accent: on this theme the arcs are the one thing that
    // says work is under way, and blue on blue would lose them.
    '--ring': '#54b64a',
    '--ring-glow': 'rgba(84, 182, 74, 0.5)',

    '--text': '#12354f',
    '--text-dim': '#3f6885',
    '--text-faint': '#7c9bb3',

    '--accent': '#2f7fc4',
    '--accent-strong': '#63aee9',
    '--accent-text': '#ffffff',
    '--accent-glow': 'rgba(47, 127, 196, 0.45)',
    // The highlight breaks at the middle rather than fading, which is what
    // makes a button on this kind of surface read as glass.
    '--action':
      'linear-gradient(180deg, #6cb6ec 0%, #4a97dc 49%, #2f7fc4 51%, #2a72b4 100%)',
    '--action-text': '#ffffff',
    '--ok': '#3f9d3f',
    '--warn': '#a5761b',
    '--danger': '#c0392b',

    '--radius': '0.3rem',
    '--font':
      'system-ui, -apple-system, "Segoe UI Variable Text", "Segoe UI", Roboto, sans-serif',
    '--font-mono': 'ui-monospace, "Cascadia Mono", "JetBrains Mono", Menlo, monospace',
  },
});
