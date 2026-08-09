import { theme } from '../contract';

/**
 * A green phosphor terminal.
 *
 * One colour, one typeface, square corners and hairline rules: everything is
 * drawn the way a character display would draw it, with brightness standing in
 * for the distinctions other themes make with hue. Nothing is translucent and
 * nothing is blurred, because a terminal has no depth — a surface is either
 * lit or it is not.
 */
export default theme({
  id: 'phosphor',
  name: 'Phosphor',
  description: 'A green terminal: one colour, monospaced, square corners.',
  scheme: 'dark',
  tokens: {
    '--backdrop': '#020805',
    '--backdrop-glow':
      'radial-gradient(130% 100% at 50% 0%, rgba(46, 200, 70, 0.07), transparent 68%)',

    '--pane': 'rgba(46, 220, 70, 0.025)',
    '--pane-blur': 'none',
    '--pane-border': '#1c5f2a',
    '--pane-shadow': 'none',
    '--pane-radius': '0',
    '--scrim': 'rgba(0, 8, 3, 0.78)',

    '--row': 'transparent',
    '--row-hover': 'rgba(78, 228, 78, 0.08)',
    '--row-selected': 'rgba(78, 228, 78, 0.13)',
    '--row-selected-border': '#4ee44e',

    '--inset': 'rgba(0, 0, 0, 0.42)',
    '--inset-border': '#1c5f2a',
    '--track': '#12401c',

    '--ring': '#4ee44e',
    '--ring-glow': 'rgba(78, 228, 78, 0.4)',

    '--text': '#45e845',
    '--text-dim': '#31b031',
    '--text-faint': '#248224',

    '--accent': '#3ad13a',
    '--accent-strong': '#6dff6d',
    '--accent-text': '#02150a',
    '--accent-glow': 'rgba(78, 228, 78, 0.35)',
    // Outlined, not filled: the border and the label carry it, the way a
    // terminal draws a control it cannot shade.
    '--action': 'transparent',
    '--action-text': '#6dff6d',
    '--ok': '#4ee44e',
    '--warn': '#c8d94a',
    '--danger': '#ff5a5a',

    '--radius': '0',
    '--font': 'ui-monospace, "Cascadia Mono", "JetBrains Mono", Menlo, Consolas, monospace',
    '--font-mono': 'ui-monospace, "Cascadia Mono", "JetBrains Mono", Menlo, Consolas, monospace',
  },
});
