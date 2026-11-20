# Snag: Neumorphism restyle

Style in four words: **soft, tinted, pressed-in surfaces.**

Tokens are the ones in the brief, mirrored one-to-one in `src/ui/theme.rs`
(`Tokens::light()`): DM Sans 400/600 at 12/13/14/17/22/28 px, neutrals at hue
220 (`bg = surface = #E5E7EB`, `sunken #D9DCE2`), one accent (ink blue
`#1F4FBF`), `--nm-d #bec0c3` / `--nm-l #FFFFFF` shadow pair, radii 10/14/22 and
pill buttons, 36 px controls, 4 px spacing grid, calm motion (160/260/420 ms),
Iconoir icons. Nothing in the UI code uses a raw colour or size outside that
file.

A dark set (`Tokens::dark()`) is derived with the same rules because Snag has a
"Match Windows / Light / Dark" setting. The brief only defines light mode, so the
dark values **need a second opinion**.

---

## 1. Audit of Snag 2.1 (before)

### Style inconsistencies

| Where | Problem |
| --- | --- |
| Everywhere | Segoe UI with egui's Ubuntu/Inter fallback: two text families, neither the brand's. |
| Everywhere | Phosphor icons drawn as font glyphs inside label strings; sizes vary (12–44 px), not on a 20 px grid. |
| Cards, rows, chips, buttons | Each built its own look: flat cards with 1 px borders, flat egui buttons, pill chips, a custom blue primary. No elevation system. |
| Colour | Hard-coded greys (`from_gray(32)`, `from_gray(252)`…) in `theme.rs`, `mod.rs`, `widgets.rs`; the accent blue was also used for progress bars, the clipboard bar and row hover, so it stopped meaning "act here". |
| Radii | 6, 8, 10, 12 and 14 px mixed with no size logic. |
| Spacing | 3, 4, 6, 8, 10, 12, 14 px gaps: no rhythm. |
| Inputs | egui default frame: a faint 1 px border well under 3:1 contrast. |
| Motion | None on custom widgets; egui's default 83 ms elsewhere. |

### Usability log (sorted by severity)

| # | Severity | Heuristic | Location | Evidence |
| --- | --- | --- | --- | --- |
| 1 | 3 | 3 User control / 5 Error prevention | Queue: Delete key and bulk **Remove** | Removing several unfinished downloads deletes their partial data at once; no confirmation, no undo. |
| 2 | 3 | Accessibility (keyboard) / 4 Standards | Queue rows, picker rows, custom icon buttons | Custom-painted controls take keyboard focus but draw no focus ring, so a keyboard user can't see where they are. AccessKit was off, so screen readers got nothing. |
| 3 | 2 | 5 Error prevention | Queue bulk **Retry** | On finished downloads it re-downloads them, creating "name (2).mp4" duplicates the user didn't ask for. |
| 4 | 2 | 4 Consistency | Selected chips, tabs, filters | Selection shown by colour alone (blue fill); no shape or icon change. |
| 5 | 2 | Accessibility (contrast) | Text inputs, combo boxes | Edges around 1.4:1 against the panel; the link box is the focal point but hard to see. |
| 6 | 2 | 1 Visibility of status | Progress bars | Running bars used the same blue as primary buttons and selection, so "in progress" and "selected" looked alike. |
| 7 | 2 | 6 Recognition over recall | Queue row actions | Icon-only (pause, cancel, open, folder) with tooltips only; fine for pause/play, less so for "download next" and "update and retry". Kept as icons for density, with tooltips and accessible names; the ⋯ menu repeats them with labels. *Needs a second opinion.* |
| 8 | 1 | 2 Match with the real world | Queue toolbar "At once" | Unclear without context; now "Downloads at once". |
| 9 | 1 | 1 Visibility of status | Settings | Changes save instantly but nothing says so. |
| 10 | 1 | 7 Flexibility | Search | The YouTube/SoundCloud switch only appears after a first search. *Needs a second opinion* on whether to show it beside the box. |
| 11 | 1 | 8 Minimalist design | Settings | One long page; a jump list may help. Not changed. *Needs a second opinion.* |
| 12 | 1 | 9 Error recovery | Failed queue rows | The error is cut to one line; full text on hover and in **Show log**. Not changed (row height is fixed for fast scrolling). |

Found while testing the restyle:

| # | Severity | Heuristic | Location | Evidence |
| --- | --- | --- | --- | --- |
| 13 | 2 | 3 User control | **Add many** dialog | The text box wasn't focused when the dialog opened, so typing or pasting went nowhere (a paste even went to the clipboard watcher behind the dialog). |
| 14 | 1 | 9 Error recovery | Preview → trim | Ticking "Only download part of it" showed a red error before anything was typed. |
| 15 | 1 | 2 Real world | Sleep/shut-down banner | The button said "Cancel"; it now says what it does: **Keep the PC on**. |

---

## 2. Changes

See the "after" screenshots in the session report. Component by component:

| Component | Before | After (all states) |
| --- | --- | --- |
| Page and panels | Grey panel, white-ish cards with borders | One surface colour (`bg`); cards are raised (`--nm-l` up-left, `--nm-d` down-right, radius `--r-lg`), no borders. |
| Primary button | Blue rectangle, radius 6 | Solid `--pri-bg` pill, white DM Sans 600, low raised shadow. Hover: lightens a touch. Pressed: pushes in. Focus: 2 px accent ring. Disabled: flat `--sunken` with `--text2` label and a `--border` edge. Loading: spinner in place of the icon. |
| Secondary button | egui default | Raised pill, `--text` label, optional 20 px Iconoir icon. Hover: shadow grows. Pressed: inset. Focus ring. Disabled: flat. |
| Tertiary (links) | egui hyperlink | `--accent-text`, underline on hover, focus ring. |
| Destructive | Primary blue | Solid `--danger` pill in confirmation dialogs. |
| Icon button | Glyph in a 30×28 box | 36 px circle: flat at rest, raised on hover, inset when pressed, focus ring, tooltip and accessible name. |
| Chip / toggle | Blue fill when on | Off: raised pill. On: accent fill **plus a check icon**. Focus ring. |
| Tabs | egui selected button | Off: flat label and icon. On: pressed-in well, accent label, weight 600. |
| Text input | egui frame | Inset well (`--sunken`, inner shadow pair) with a 1 px `--border-strong` edge (3.3:1 against the page); focus turns the edge into a 2 px accent ring. Optional leading icon. |
| Checkbox | egui | Inset 18 px box with a `--border-strong` edge; on = accent fill with a white check. |
| Progress | Flat bar, blue | Inset track; fill is `--text2` while running, `--success-text` done, `--danger` failed, `--warning-text` paused. Each also has a text status. |
| Lists (picker, results) | Bordered frame | Inset well; rows flat, ticked rows tinted with `--accent-soft` plus the check box. |
| Queue rows | Hover tint | Flat; hover raises the row slightly; selected rows press in and gain an accent marker. |
| Banners (clipboard, power, download hours) | Coloured boxes | Raised `--r-md` cards with a leading icon; tone shown by icon colour and words, not background. |
| Toasts | Dark box | Raised surface card with `--shadow-overlay` and a 1 px `--border` edge. |
| Dialogs, menus, tooltips | egui defaults | `--surface`, `--shadow-modal` / `--shadow-overlay`, radii `--r-lg` / `--r-md`. |
| Type | Segoe UI | DM Sans 400 for UI and body, 600 for headings, labels and buttons; 12/13/14/17/22 px; headings tracked -0.01 em. |
| Icons | Phosphor glyphs | Iconoir SVGs rendered by `resvg`, 20 px (16 px inside chips), tinted from tokens. |
| Motion | None | Hover and press blend over 160 ms; egui transitions 260 ms; all off when Windows "Show animations" is off. |

Usability fixes from the log: #1 confirmation before removing unfinished
downloads ("Remove 3 downloads? 2 of them haven't finished…", Danger button,
"Keep them" to back out), #2 focus rings on every custom control (shown while
the keyboard drives, like `:focus-visible`) plus AccessKit and accessible names,
#3 bulk Retry skips finished downloads and says so, #4 selected states change
shape or icon as well as colour, #5 3:1 input edges, #6 neutral running bars,
#8 "Downloads at once", #9 "Changes save as you make them.", #13 Add many
focuses its text box, #14 the trim fields show a quiet hint until something is
typed, #15 "Keep the PC on".

Screens (after): `docs/screens/01-preview.jpg` … `15-dark-settings.jpg`.

---

## 3. Self-check

| Rule | Result |
| --- | --- |
| 1 Hierarchy | One focal point per screen: the link box on Downloads, the search field on History, the follow box on Subscriptions. The accent appears only on the primary action, selections and focus. |
| 2 Type | DM Sans only for UI (Consolas stays for the log and the template field, which are code). Two weights. 12–22 px from the scale. Hints are capped at about 75 characters per line. |
| 3 Colour | Tinted neutrals, one accent, semantic status colours. No gradients or glow. Status always has words too. |
| 4 Space | 4 px grid (4/8/12/16/24/32). |
| 5 Shape and depth | Radii by size: 10 small controls, 14 rows/menus/banners, 22 cards, pill buttons. Three levels: inset, raised low (controls), raised mid (cards). |
| 6 Components | One look per job, every state above. Snag has no phone UI, so the phone rules don't apply. |
| 7 Icons | Iconoir only (v7.12.2, MIT, licence in `assets/iconoir/LICENSE`), 20 px grid, labels beside icons except universal ones (play, pause, close, more). |
| 8 Motion | 160 ms feedback, 260 ms transitions, reduced motion respected. |
| 9 Voice | Action-named buttons ("Download 2 videos", "Add 3 links", "Follow"). Errors say what happened and what to try. |
| 10 Accessibility | Text 4.5:1 and parts 3:1 (checked below), focus rings, keyboard path, AccessKit names. Controls are 36 px on desktop; the 44 px touch rule targets phones. |

Contrast, measured (light, on `--bg`): `--text` 13.9:1 · `--text2` 5.8:1 ·
`--accent-text` 5.8:1 · `--danger-text` 5.5:1 · `--success-text` 5.4:1 ·
`--warning-text` 5.3:1 · white on `--pri-bg` 7.2:1 · white on `--danger`
6.6:1 · `--text` on `--accent-soft` (ticked rows) 11.7:1. On `--sunken`
(inside fields) every text colour stays at 4.8:1 or more. The input edge
`--border-strong` is 3.3:1 against the page around it (2.9:1 against the field's
own fill), so the field outline passes 3:1 where it meets the page.

Dark (derived, on `#2A2F37`): text 11.2:1 · text2 6.2:1 · accent text 6.3:1 ·
input edge 3.7:1 · white on the primary fill 4.8:1.

Checklist: no default fonts, no purple-to-blue gradients, no centred hero with
three icon cards, radius and shadow vary by role, no emoji icons, no
hand-drawn icons outside Iconoir, no placeholder names.

---

## 4. Test with people

This review can't tell whether the design works for real people. Five tasks to
watch someone do, without help:

1. "Download the audio of this video as an MP3, but only from 1:00 to 2:30." (Preview, audio chips, trim.)
2. "Search for Blender's open movies, pick the two shortest and download them." (Search, picker, primary action.)
3. "Pause everything that's downloading, then start just one of them again." (Pause all, row actions, status.)
4. "Five downloads failed. Retry them, then remove the ones that still fail." (Failed filter, multi-select, bulk actions, the new confirmation.)
5. "Make Snag only download at night and put the PC to sleep when it's done." (Settings → download hours, When done, the countdown.)

Watch for: whether pressed and raised states read as "on" and "off" without the
accent, whether people find the eye button in results, and whether the inset
link box reads as "type here".
