# Snag 2.3: Command console

This restyle changes only the style layer of Snag. Functions, content, structure and flows are the same as in 2.2. The previous Neumorphism notes and screenshots are in `archive/neumorphism/`.

## 1. The style, and the tokens

**The style in four words:** pale lines on black.

One ice-blue line colour (`--accent` #AACFD1) carries everything on near-black: text, hairlines and fills, each at a different strength.

- **Modules** have no box. Each one is a hairline with a tick at each end, plus a small, widely tracked uppercase title.
- **Overlays and the work area** (dialogs, menus, toasts, the main work area) sit in a 1 px frame with two clipped corners.
- **Buttons** are keys: thin outlines that fill solid when pressed or chosen.
- **Values and inputs** are set in Fira Code.

**Where the tokens live.** Every token is a field of `Tokens` in `src/ui/theme.rs`, under the brief's name. Nothing else in the UI hard-codes a colour, size or duration. `Tokens::dark()` holds the brief's values exactly.

Notes on the mapping:

- **Weights.** egui picks weights by font family, so the weight tokens become families:
  - `--fw-label` (600) is `theme::bold()`, Saira SemiBold.
  - `--fw-head` (500) is `theme::head()`, Saira Medium.
  - Body text uses Saira Regular (400).
- **Headings.** `--heading-scale` and `--heading-tracking` are applied by `widgets::title_text` and `widgets::heading_text`.
- **Shadows.** `--shadow-*` are `Shadow::NONE`.
- **Console extras.** `--dk-line`, `--dk-dim`, `--dk-fill` and `--dk-fill2` are used as:
  - module hairlines;
  - the progress track;
  - hover;
  - pressed and open states.
- **Light theme.** `Tokens::light()` is derived for Snag's existing Theme setting and is not part of the brief. It uses one ink colour (#163D42) as both text and accent, on pale neutrals at hue 216, and passes the same contrast test.
- **Default theme.** It is now **Dark**, because the brief defines a dark style. "Match Windows" and Light are still offered.
- **Fonts and icons.**
  - Fonts are bundled from the upstream projects: Saira from Omnibus-Type, Fira Code from tonsky/FiraCode. Both are under the OFL, and the licence files sit in `assets/fonts/`.
  - Icons are the official Phosphor Thin SVGs from `@phosphor-icons/core` 2.1.1, under the MIT licence. The licence is in `assets/phosphor/LICENSE`.

## 2. Audit

The 2.2 build was audited against the brief. Problems are listed by severity (0 to 4).

### Style inconsistencies, from 2.2 to the brief

| Area | 2.2 (Neumorphism) | Brief |
| --- | --- | --- |
| Type | DM Sans, 14 px body | Saira 13 px, uppercase tracked headings, Fira Code for values |
| Colour | Grey surface, ink-blue accent | Near-black, one ice-blue line colour |
| Shape | Pills and 22 px radii | 2 to 4 px radii; clipped corners on overlays |
| Depth | Paired soft shadows, inset wells | No shadows; hairlines only |
| Icons | Iconoir | Phosphor Thin |
| Density | 36 px controls, 48 px rows | 28 px controls, 36 px rows (compact) |
| Motion | 160 / 260 ms | 90 / 140 / 220 ms |

### Usability log

| # | Where | Heuristic | Evidence | Sev | Done |
| --- | --- | --- | --- | --- | --- |
| 1 | Search results | 6 Recognition | With compact 36 px rows the channel and view-count line disappeared (egui hides rows that overflow). | 3 | Row height is measured from the real font heights (`picker::row_height`). |
| 2 | Chosen rows (queue, picker) | 10 Accessibility | `--text2` on `--accent-soft` is 3.9:1. | 2 | Secondary text on a chosen row switches to `--text` (`widgets::sub_color`). |
| 3 | Every icon | 1 Visibility | Phosphor Thin strokes are 0.5 px at 16 px and render half-strength. | 2 | Coverage gain of 1.6 at raster time (`icons::INK_GAIN`). Shapes are unchanged. |
| 4 | Video titles, file names | 2 Real world | Uppercase headings would rewrite people's titles and names. | 2 | `widgets::content_title` keeps the original case. Only Snag's own headings are uppercase. |
| 5 | Lists on near-black | 1 Visibility | The idle scroll handle was invisible, so there was no cue that a list scrolls. | 2 | The handle uses the line colour and stays at 35 % when idle. |
| 6 | Destructive keys | 10 Accessibility | White isn't a token, and `--text` on `--danger` is 3.9:1. | 2 | Danger is an outline in `--danger` with a `--danger-text` label (8.7:1). Pressed, it fills `--danger-text` with an `--on-accent` label (8.3:1). |
| 7 | Monospace file names | 8 Aesthetic | A full-width colon (from yt-dlp's file names) showed as a missing-glyph box. | 1 | File names are scanned for script fonts, and the CJK fallback now covers monospace too. |
| 8 | Slider rails | 8 Aesthetic | The `--accent-soft` trailing fill on `--dk-dim` was nearly invisible: noise with no meaning. | 1 | Trailing fill is off. The value sits in its own box beside the rail. |
| 9 | Combo-box dropdowns | 4 Consistency | egui's built-in dropdown frame can't take clipped corners. | 1 | It keeps a 1 px square frame in `--border-strong`. Menus Snag draws itself are clipped. |
| 10 | All controls | 10 Accessibility | 28 px controls are below the brief's 44 px touch rule. | 1 | Kept. Snag is a mouse-driven desktop app, and 28 px clears WCAG 2.2 AA's 24 px target. *Needs a second opinion.* |
| 11 | Big numbers | Rule 2 | "Big numbers are light" would add a fourth weight. | 0 | The rule wins: 400 / 500 / 600 only. Snag has no big numbers anyway. |

Still open from the 2.2 audit, each marked *needs a second opinion*:

- icon-only row buttons;
- the search-site switch;
- the long Settings page.

## 3. What changed, per component

Each item gives the before (2.2) and the after (2.3).

### Keys (buttons)

**Primary**
- **Before:** a solid blue pill with a soft shadow.
- **After:** a solid ice key (`--pri-bg`) with a dark label.
  - Hover dims it 14 %.
  - Pressed dims it 28 % and moves the label down 1 px (key travel).
  - Focus adds a 2 px accent ring.
  - Disabled is a dim outline with a `--border-strong` label.
  - Loading shows a spinner in place of the icon.

**Secondary**
- **Before:** a raised pill.
- **After:** an outline key in `--border-strong`.
  - Hover adds a `--dk-fill` wash and brightens the edge to `--text`.
  - Pressed lights the key solid.
- Labels are uppercase, 600 weight, lightly tracked.

**Quiet**
- **Before:** blue text.
- **After:** a bare label.
  - Hover adds a `--dk-fill` wash.
  - Pressed fills with `--dk-fill2`.

**Danger**
- **Before:** a solid red pill.
- **After:** an outline key in `--danger` with a `--danger-text` label.
  - Pressed fills with `--danger-text`.

**Icon keys**
- **Before:** round.
- **After:** square.
  - Bare at rest.
  - Outlined on hover.
  - Lit when pressed.

### Choices

- **Chips:** outline keys; a chosen chip is lit solid and shows a check mark. Notes that are values (sizes, frame rates) are set in Fira Code; notes that are words are not.
- **Checkboxes:** an outlined square. On, it shows a lit inner square, so the state changes shape and not just colour.
- **Segmented controls:** joined keys with hairline dividers. The chosen segment is lit.
- **Tabs:** uppercase and tracked at .12 em.
  - The chosen tab is full strength with a 2 px accent bar under it.
  - A hover shows a hairline.

### Inputs

- Monospace text in a sunken well with a 1 px `--border-strong` edge (4.1:1 on `--sunken`).
- Hover changes the edge to `--text2`.
- Typing gives a 2 px accent edge.

### Containers

- **Cards and settings sections** become modules: a hairline with end ticks, then an uppercase tracked title.
- **Dialogs, menus (the ⋮ menu and right-click menus) and toasts:** a 1 px frame with two clipped corners and no shadow. The page is dimmed behind dialogs.
- **Main work area:** the same clipped frame, on `--surface`.
- **Banners:** the one boxed element on a page (1 px `--border-strong`), so system notices stand out.

### Lists

- Rows sit on hairlines.
- Hover adds a faint wash.
- A chosen row gets an `--accent-soft` fill and a 2 px accent marker.
- Keyboard focus shows a 2 px ring.

### Feedback

- **Progress meters:** 4 px, square, on `--dk-dim`.
  - Running: `--text2`.
  - Done: `--success-text`.
  - Failed: `--danger-text`.
  - Paused: `--warning-text`.
- **Status lines:** set in Fira Code, and they always say the state in words.
- **Status pips:** square.

### Type and motion

- **Type:** Snag's own headings are uppercase Saira 500, tracked .12 em. Content titles keep their case. Values (speeds, sizes, times, paths, versions) are set in Fira Code.
- **Motion:** hover and press take 90 ms (`--dur-fast`); egui's own transitions take 140 ms (`--dur-base`). With Windows animations off, everything is instant.

### The rest

- **Bookmarklet page:** rewritten with the same tokens as CSS custom properties.
- **About:** the credits now name Phosphor, Saira and Fira Code.

## 4. Self-check

**The ten rules**

1. **Hierarchy.** One lit key per screen: Download, Add N links, or Follow.
2. **Type.** Saira and Fira Code only, on the 12 / 12.5 / 13 / 15 / 19 / 24 scale, with three weights.
3. **Colour.** One accent; semantic colours only for status, always paired with words or an icon. No gradients and no glow.
4. **Space.** Everything is built in 4 px steps (`Tokens::s`).
5. **Shape and depth.** Radii are 2 to 4 px. There are no shadows. Modules have no box. Overlays use clipped corners.
6. **Components.** Every component has a state for default, hover, pressed, focus and disabled, plus loading or error where that applies (section 3).
7. **Icons.** Phosphor Thin only, with the licence kept. Icon-only keys have an accessible name and a tooltip.
8. **Motion.** As in section 3. Reduced motion is respected.
9. **Voice.** Unchanged from 2.2: plain, action-named buttons.
10. **Accessibility.** Contrast is below; there's a visible focus ring and a full keyboard path. The touch target size is item 10 in the usability log.

**The banned-pattern list:** no default fonts, no purple-to-blue gradients, no centred hero, no identical radius-and-shadow on everything, no emoji, no hand-drawn SVGs, no placeholder names. The test content is Blender's open movies.

**Contrast, measured, on `--bg`.** The test `theme::tests::tokens_keep_their_contrast` asserts every one of these on every surface they sit on, in both themes.

| Pair | Dark | Light |
| --- | --- | --- |
| Text | 11.9 | 10.4 |
| Secondary text | 6.2 | 5.7 |
| Danger text | 8.7 | 6.6 |
| Success text | 10.3 | 5.9 |
| Warning text | 10.6 | 6.3 |
| Input and key edges | 4.5 (4.1 on the input fill) | 3.7 (3.4) |
| Primary key label | 11.3 | 10.9 |

**Not checked:**
- a screen reader reading the custom controls;
- the drop overlay and the sleep/shut-down banner, live;
- the light theme, which needs a second opinion.

## 5. Five tasks to test with real people

1. Download one video as an MP3, but only from 1:00 to 2:30.
2. Search for Blender's open movies and download the two shortest.
3. Pause everything that's downloading, then restart just one.
4. Retry the failed downloads, then remove the ones that fail again.
5. Set Snag to download only at night and sleep the PC when it's done.

## Screens (`docs/screens`)

| File | What it shows |
| --- | --- |
| 01-preview | Preview |
| 02-queue-states | Downloading, waiting, failed and finished |
| 03-multi-select | Several rows chosen |
| 04-remove-confirm | The remove dialog |
| 05-search-picker | The search picker |
| 06-add-many | The Add many dialog |
| 07-settings | Settings |
| 08-settings-more | More of Settings |
| 09-history | History |
| 10-subscriptions | Subscriptions |
| 11-row-menu | A row menu |
| 12-focus-ring | The keyboard focus ring |
| 13-trim | Trim |
| 14-light-queue | The queue in the derived light theme |
| 15-light-settings | Settings in the derived light theme |
