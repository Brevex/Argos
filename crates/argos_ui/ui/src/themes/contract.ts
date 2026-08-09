/**
 * The contract between the base layout and a theme.
 *
 * A theme is presentation and nothing else: a name, a colour scheme, and a
 * value for every design token the layout uses. It has no markup, no
 * behaviour, and no access to anything the engine said. Switching one changes
 * custom properties on the document root — no component is remounted, so a
 * scan in progress and everything on screen survive the switch.
 *
 * `Record<ThemeToken, string>` is total. **A theme that omits a token does not
 * compile**, which is the same kind of guarantee the rest of this project gets
 * from its types rather than from a checklist.
 */

/**
 * Every custom property the base layout reads.
 *
 * Adding one here breaks every theme until each supplies it — deliberately.
 * The alternative is a token that silently falls back to nothing in the themes
 * nobody remembered to update.
 */
export const THEME_TOKENS = [
  // The page behind everything, and the light that gives it depth.
  '--backdrop',
  '--backdrop-glow',

  // Panes. `--pane` is a translucent fill; `--pane-blur` is the backdrop
  // filter behind it. A theme with no translucency sets the fill opaque and
  // the blur to `none`, and the layout is unchanged.
  '--pane',
  '--pane-blur',
  '--pane-border',
  '--pane-shadow',
  '--pane-radius',
  '--scrim',

  // Rows in the drive table, and the one that is selected.
  '--row',
  '--row-hover',
  '--row-selected',
  '--row-selected-border',

  // Inset surfaces: the destination field, the progress-ring track.
  '--inset',
  '--inset-border',
  '--track',

  // The progress arcs. Separate from the accent because the thing that says
  // "work is happening" and the thing that says "this is the button" are not
  // required to be the same colour, and in some themes they are not.
  '--ring',
  '--ring-glow',

  // Text.
  '--text',
  '--text-dim',
  '--text-faint',

  // Meaning. These name a role, never a colour: a theme decides what "danger"
  // looks like, the layout only decides what is dangerous.
  '--accent',
  '--accent-strong',
  '--accent-text',
  '--accent-glow',

  // The one button that starts and stops a scan. Its fill is a whole
  // background value rather than a colour, so a theme can make it a gloss, a
  // flat block or an outline without the layout knowing which.
  '--action',
  '--action-text',
  '--ok',
  '--warn',
  '--danger',

  // Shape and rhythm.
  '--radius',
  '--font',
  '--font-mono',
] as const;

/** One custom property the base layout reads. */
export type ThemeToken = (typeof THEME_TOKENS)[number];

/** A theme module, as `themes/<id>/index.ts` default-exports one. */
export interface ThemeModule {
  /** Stable identifier; also the directory name and what is persisted. */
  readonly id: string;
  /** Name shown in the theme picker. */
  readonly name: string;
  /** One line saying what it is for, shown under the name. */
  readonly description: string;
  /**
   * What the browser should assume about the page's own colours, so form
   * controls and scrollbars match rather than fighting the theme.
   */
  readonly scheme: 'dark' | 'light';
  /** A value for every token. Total by construction. */
  readonly tokens: Readonly<Record<ThemeToken, string>>;
}

/**
 * Helper that gives a theme module its type without widening it.
 *
 * Themes call this instead of annotating, so a missing token is reported at
 * the property that is missing rather than as a mismatch on the whole object.
 */
export function theme(module: ThemeModule): ThemeModule {
  return module;
}
