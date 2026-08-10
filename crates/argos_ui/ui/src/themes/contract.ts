/**
 * The contract between the base layout and a theme.
 *
 * A theme is presentation and nothing else: a name, a colour scheme, a value
 * for every design token the layout uses, and the artwork for a fixed set of
 * glyphs. It has no markup of its own, no behaviour, and no access to anything
 * the engine said.
 *
 * **Nothing here is a measurement.** Sizes, spacings and positions belong to
 * the layout and are the same under every theme, so a control cannot sit in
 * one place on one theme and somewhere else on another. The one exception is
 * the frame band, which is the frame — a theme that has no frame gives it no
 * width. Everything else a theme decides is colour, fill, edge, shadow,
 * corner, texture, typeface and artwork. Switching one changes custom properties on
 * the document root; no component is remounted, so a scan in progress and
 * everything on screen survive the switch.
 *
 * `Record<ThemeToken, string>` and `Record<ThemeIcon, string>` are total. **A
 * theme that omits either does not compile**, which is the same kind of
 * guarantee the rest of this project gets from its types rather than from a
 * checklist.
 */

/**
 * Every custom property the base layout reads.
 *
 * Adding one here breaks every theme until each supplies it — deliberately.
 * The alternative is a token that silently falls back to nothing in the themes
 * nobody remembered to update.
 */
export const THEME_TOKENS = [
  // The frame band itself — the surface between the window's outer edge and
  // its client area, which the title bar is part of rather than sitting on
  // top of. Where a theme leaves it translucent, the real desktop shows
  // through it, because the window is transparent outside what it paints.
  '--backdrop',
  '--backdrop-glow',

  // The window itself: its edge, its corner and the shadow it casts. The
  // application draws its own frame, so this is the frame.
  //
  // `--window-inset` is how far the client area sits inside that edge — the
  // width of the frame band itself. Zero makes the frame a hairline; a real
  // measure makes it the wide translucent border a window of a certain
  // generation had, with the desktop showing through it.
  '--window-border',
  '--window-radius',
  '--window-shadow',
  '--window-inset',

  // The client area: the surface the controls actually sit on, inside the
  // frame. A theme that wants no distinction between frame and client gives
  // this no fill and no border.
  '--main-surface',
  '--main-border',
  '--main-shadow',
  '--main-radius',

  // The title bar across the top of that frame.
  '--titlebar',
  '--titlebar-border',
  '--titlebar-text',

  // Minimise, maximise and close. They sit in a group, and a theme decides
  // whether that group is a glass strip or nothing at all: give the group and
  // the dividers no colour and three bare glyphs are what is left.
  '--winbtn-group',
  '--winbtn-group-border',
  '--winbtn-group-radius',
  '--winbtn-divider',
  '--winbtn-text',
  '--winbtn-hover',
  '--winbtn-close-hover',
  '--winbtn-close-hover-text',
  '--winbtn-stroke',

  // Panes. `--pane` is a translucent fill; `--pane-blur` is the backdrop
  // filter behind it. A theme with no translucency sets the fill opaque and
  // the blur to `none`, and the layout is unchanged.
  '--pane',
  '--pane-blur',
  '--pane-border',
  '--pane-shadow',
  '--pane-radius',
  '--scrim',

  // The sheet the drive table and the destination field share.
  //
  // Whether they share one at all is a theme's decision: giving this no fill,
  // no border and no shadow leaves the two controls sitting directly on the
  // client area as two separate things, which is what some designs want and
  // others do not.
  '--form-pane',
  '--form-pane-border',
  '--form-pane-shadow',

  // Rows in the drive table, and the one that is selected. `--dot-idle` is the
  // ring on a row that is not selected; a theme that shows nothing there gives
  // it no colour.
  '--row',
  '--row-hover',
  '--row-selected',
  '--row-selected-border',
  '--dot-idle',

  // Inset surfaces: the destination field, the progress-ring track.
  '--inset',
  '--inset-border',
  '--track',
  '--track-edge',

  // The progress arcs. Separate from the accent because the thing that says
  // "work is happening" and the thing that says "this is the button" are not
  // required to be the same colour, and in some themes they are not.
  //
  // `--ring-pulse` is a highlight that sweeps along the filled part of the
  // arc, the way a progress bar of a certain vintage does. A theme that wants
  // none sets its duration to `0s`, which is what stops the animation from
  // running at all rather than merely hiding it.
  '--ring',
  '--ring-highlight',
  '--ring-shadow',
  '--ring-glow',
  '--ring-cap',
  '--ring-pulse',
  '--ring-pulse-duration',

  // The texture of a lit surface, laid over the fill of the panes, the table,
  // the field, the statistics strip and the progress arcs — and over nothing
  // else. A display that drew in rows leaves rows; the frame, the title and
  // the window edge were never on that display and stay crisp. `none` for a
  // theme whose surfaces are just surfaces.
  '--scanlines',

  // The bloom a phosphor dot leaves around whatever it lights: a `text-shadow`
  // applied to every piece of text. `none` for a theme with no phosphor.
  '--text-glow',

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
  '--action-border',
  '--action-shadow',

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

/**
 * Every glyph a theme draws.
 *
 * Artwork, not markup: each is the inside of an `<svg>` whose viewBox the
 * layout fixes, so a theme chooses how a drive is drawn and cannot choose that
 * a drive is drawn somewhere else.
 */
export const THEME_ICONS = [
  /** The drive in each row of the table. `viewBox="0 0 20 16"`. */
  'disk',
  /** Minimise, maximise, close. `viewBox="0 0 12 12"`, stroked in currentColor. */
  'minimise',
  'maximise',
  'close',
] as const;

/** One glyph a theme draws. */
export type ThemeIcon = (typeof THEME_ICONS)[number];

/** A theme module, as `themes/<id>/index.ts` default-exports one. */
export interface ThemeModule {
  /** Stable identifier; also the directory name and what is persisted. */
  readonly id: string;
  /** Name shown in the theme picker, and the only thing it shows. */
  readonly name: string;
  /**
   * What the browser should assume about the page's own colours, so form
   * controls and scrollbars match rather than fighting the theme.
   */
  readonly scheme: 'dark' | 'light';
  /** A value for every token. Total by construction. */
  readonly tokens: Readonly<Record<ThemeToken, string>>;
  /** Artwork for every glyph. Total by construction. */
  readonly icons: Readonly<Record<ThemeIcon, string>>;
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
