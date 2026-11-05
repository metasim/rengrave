# Plan: System fonts and a user font library

Status: draft 2 for review, revised after owner feedback. Nothing has been implemented or
prototyped; the claims below come from reading the code and the crate sources in the Cargo registry.

Out of scope: text layout (kerning, shaping, spacing). That was done afterwards; see D19 in
`docs/DECISIONS.md`.

## Goal

Replace the "pick a font file from a folder" flow with a font picker offering two libraries:

- **System fonts:** every font installed on the machine.
- **User fonts:** individual font files (CXF, TTF, OTF, TTC) the user has added to a library, managed
  from a new application settings dialog.

macOS first; Windows and Linux must work later without redesign. Reuse what egui already ships to
keep dependencies down.

## Owner decisions (from review of draft 1)

| # | Decision |
|---|---|
| Q1 | Keep CXF and other individual font files, but as a **user font library** managed in a new application settings dialog. No "choose folder" control. The picker shows the entries under "User fonts", separate from "System fonts". |
| Q2 | No backward compatibility with F-Engrave settings files. (The message was cut off after "Just"; I read it as "no compatibility needed, and don't plan for machining/tool settings yet." Please correct me if there was more.) |
| Q3 | `fontique` is a dependency of `rengrave_core`, not only the GUI. |
| Q4 | Avoid loading huge fonts by accident without limiting what users can select. See "Large fonts" below. |
| Q5 | Prefer bundling no fonts; if one is bundled it must have a very permissive license. See "Default font" below. |
| Q6 | My reading of Q2 (no F-Engrave compatibility, ignore machining/tool settings for now) is correct. |
| Q7 | Application settings use `serde` with `serde_norway` (YAML), not TOML and not a hand-rolled format. |
| Q8 | Default font is the system sans-serif; no bundled engraving font. |
| Q10 | Application settings live in a YAML file in the platform config directory, shared by the GUI and CLI, located with a cross-platform directories crate (preferring whatever egui uses). |
| Q12 | Replace the "Show all characters" checkbox with a third Source mode, `[Text | Font Page | Image]`. See "Source panel redesign". |
| Q13 | The font picker is a shared row above the mode-specific controls (shown for Text and Font Page, hidden for Image). |
| Q14 | "Manage fonts..." in the picker opens the settings dialog on the Fonts page. |
| Q9 | 2,000 glyphs per page is too many. Use roughly a quarter of that by default, with Unicode blocks as the natural unit and the user able to choose which blocks to show. See "Large fonts". |

## Current state

- `crates/rengrave_core/src/font.rs` reads CXF files and TTF/OTF bytes (via `ttf-parser`, to be replaced by `skrifa`) and
  reduces them to straight strokes. `BuiltinFont` embeds Ubuntu Light and Hack through
  `epaint_default_fonts`.
- `Job::load_font` (`job.rs:53`) resolves `Settings::font_file` as: empty, then a built-in name,
  then `font_dir/font_file`.
- The GUI scans `font_dir` with `Font::list_files` (`app.rs:377`) and shows a filtered combo box
  (`inspector.rs:112-180`). The CLI has `--font <path|builtin>` and a `fonts` subcommand.
- The only persisted state is the job settings, saved as text through `eframe` storage
  (`main.rs:20`). There is **no application-level settings store and no settings dialog yet**, and
  no `serde` in the workspace crates.
- Only the outline-to-stroke step needs font bytes. `Design::layout` and V-carve consume a `Font` of
  strokes and are unaffected.

## What egui already gives us

| Finding | Consequence |
|---|---|
| `eframe` 0.37's default `system_fonts` feature pulls in `egui_system_font_provider` (already in `Cargo.lock`), built on `fontique` 0.11.1 and `skrifa` 0.44. | `fontique` is already compiled into the GUI. Pin the same version in core to avoid a duplicate. |
| `egui_system_font_provider` only finds a fallback font per character. It has no API to list families. | We cannot use it as the picker. We use `fontique` directly, the engine it uses. |
| `fontique` backends: CoreText (macOS), DirectWrite (Windows), fontconfig via `dlopen` (Linux, so no link-time `libfontconfig`), Android. | One code path covers all platforms. |
| `fontique::Collection` offers `family_names()`, `family_by_name()`, per-face `FontInfo` (weight, width, style, variable axes, source, collection index), and `FontInfo::load` returns a memory-mapped blob. Also `load_fonts_from_paths` and `register_fonts`. | Covers the system library, the face picker, TTC indices, and test fixtures. |
| `fontdb` and `font-kit` are not in the lockfile. | Either would be a new dependency; no reason to. |

**Outline reader: `skrifa` replaces `ttf-parser` in core.** `epaint` 0.37 and `fontique` already use
`skrifa` (and `read-fonts`), so core adds no new crate and drops its direct `ttf-parser`
dependency. `OutlineSink` changes from `ttf_parser::OutlineBuilder` to `skrifa::outline::OutlinePen`
(same five methods); character lookup uses the font's Unicode charmap, scaling uses units per em,
and variable-font weight uses a variation location. The existing curve-flattening and font tests
guard the change. `fontique` and `skrifa` are MIT/Apache-2.0, compatible with GPL-3.0-or-later.

## Design points common to every option

1. **Font identity.** A new `FontRef` type in core replaces the `font_dir` + `font_file` pair in
   `Settings`:
   - `system:<Family>` (later `system:<Family>;<Face>`)
   - `user:<Name>` (a name in the user library)
   No F-Engrave compatibility is kept, so `font_dir`, the `fontdir` key, and folder scanning are
   removed rather than deprecated. The CLI gets `--font system:Helvetica` and `--font-file <path>`
   for an ad hoc file.
2. **Core API.** A `FontCatalog` type in `rengrave_core` (a `system-fonts` cargo feature, default
   on, lets library users opt out) owns the `fontique` `Collection` and `SourceCache`. Methods:
   `system_families()`, `user_fonts()`, `add_user_font(path)`, `remove_user_font(name)`,
   `load(&FontRef, ...) -> Font`. Methods on types, per repo rules.
3. **User font library storage.** The library is application state, not job state: it holds
   absolute paths (and a face index for `.ttc`) with a display name. It needs an application
   settings store, which does not exist yet. Decided: `serde` with `serde_norway` (YAML), as an
   `AppSettings` type in core so the GUI and CLI share it. The GUI keeps it in `eframe` storage or a
   YAML file `settings.yaml` in the platform config directory, read by both the GUI and the CLI.
   Directory lookup: `eframe` 0.37 does not use a directories crate at runtime. It has its own
   per-OS code for its storage directory, and its tests check that code against
   `directories::ProjectDirs::from("", "", app_id)`. So `directories` is the crate egui's own
   authors treat as the reference. Use it with the same arguments, so the config directory sits in
   the same per-app folder as `eframe`'s (on macOS and Windows it is the same root; on Linux
   `eframe` uses the data directory, `~/.local/share`, while config belongs in `~/.config`, which is
   what `ProjectDirs::config_dir` returns). `directories` is not in `Cargo.lock` today; it is small and
   MIT/Apache-2.0. Job settings keep their existing text format; only application settings use YAML.
   Not yet checked: `serde_norway`'s current version, maintenance state, and license (I recall it
   as a maintained fork of the archived `serde_yaml`); confirm before adding. Neither `serde` nor
   `serde_norway` is a dependency of the workspace crates today (`eframe` pulls `serde` and `toml`
   into the GUI only). A job referencing `user:Name` that is missing on another machine gets the
   missing-font handling below.
4. **Application settings dialog.** A new dialog (menu: Rengrave > Settings) whose first and, for
   now, only page is "Fonts": a list of user fonts with Add (file chooser accepting CXF, TTF, OTF,
   TTC), Remove, and a validity indicator (file missing, no outlines). Future machining and tool
   libraries would become further pages; nothing is designed for them now.
5. **Picker.** One combo box with a filter box and two groups, "User fonts" first (usually short),
   then "System fonts". The current selection always shows.
6. **Enumeration cost.** Scanning takes hundreds of milliseconds on a populated machine. Run it on
   a background thread at startup; the picker shows "Loading system fonts..." until done. The CLI
   scans lazily, only when a `system:` name is used.
7. **Fonts that cannot be engraved.** Bitmap and color-only faces (Apple Color Emoji) have no
   outlines. `from_outline_font` already reports this; the picker hides or grays them. Whether
   `skrifa` accepts everything CoreText lists (`.dfont`, odd `.ttc`) is unverified. The picker is
   built to tolerate failure: a face that fails to open is hidden, and a test over the installed
   fonts reports (does not fail on) the count so we learn how common it is.
8. **Licensing.** Engraving outlines of a user's installed or added font is subject to that font's
   license, which is the user's concern. The README should say so. rengrave bundles nothing new.
9. **Tests.** Never depend on the host's fonts. Build the catalog with system fonts disabled and
   register fixture fonts. GUI `egui_kittest` tests do the same.

## Source panel redesign (Q12)

Today the Source panel (see `inspector.rs:112-135`) has `[Text | Image]`, then the text box, the
font combo with a Folder button, and two checkboxes. "Show all characters" quietly replaces the
user's text with a sample of the font's characters; it is not a view option but a different thing
to engrave. "Extended characters" is a loading detail that users should not have to understand.

Proposal, matching your suggestion: `InputType` gains a third variant, giving
**`[Text | Font Page | Image]`**.

- **Font row (shared):** the font picker (User fonts / System fonts groups) appears above the
  mode-specific controls and is shown for Text and Font Page, hidden for Image. The Folder button
  is removed; the picker footer has a "Manage fonts..." link that opens the new settings dialog.
- **Text mode:** the text box, as now. The "Extended characters" checkbox is removed (characters
  load on demand, see "Large fonts").
- **Font Page mode:** replaces the text box with a Unicode block chooser (only blocks the font
  covers, each with its glyph count) and, for blocks over the page size, Previous/Next. The
  engraved output is the glyphs of that block, laid out in rows as `show_font_index` does now.
  This makes the "see what the font contains" workflow direct: choose the font, choose Font Page,
  browse blocks.
- **Image mode:** unchanged.
- **Settings impact:** `show_font_index` and `extended_chars` are removed from `Settings`. Font Page
  adds the chosen block and page number. Because these decide what gets cut, they are part of the
  job like the text is, so they save with the job's settings file and restore when it is reopened.
  (My earlier "per job or per session" question reduces to this: the app remembers the choice
  exactly as it remembers the text. I'd keep it simple and not treat it separately.)

Open design details I'd settle in the implementation PR: which block is selected when the font lacks
Basic Latin, and whether "Font Page" output is allowed to proceed to V-carve (F-Engrave allows it, so
yes).

## Large fonts (Q4)

Today the program loads Latin-1 (255 code points) unless the "Load characters beyond Latin-1"
checkbox is set, which loads every code point in the font. My proposal removes the need for that
trade-off:

- **Always load Latin-1 plus whatever characters appear in the text.** Typing a Japanese word in a
  CJK font loads only those glyphs. Cost is proportional to what the user engraves, and most users
  never pay for a large font. This replaces the `extended_chars` checkbox for non-CXF fonts.
- **"Show all characters" works one Unicode block at a time.** Unicode blocks are the natural break
  point: most script blocks hold 128 or 256 code points (Basic Latin 128, Latin-1 Supplement 128,
  Latin Extended-A 128, Greek 144, Cyrillic 256), so a block is a sensible unit to render. The
  view has a block chooser listing only blocks the font actually covers, each with its glyph count
  (from the cmap, which is cheap), so the user picks which "code page" to see.
  - **Default:** Basic Latin plus Latin-1 Supplement (256 glyphs), the same set the program loads
    today.
  - **Hard cap of 500 glyphs per page** (about a quarter of the earlier 2,000). A few blocks exceed
    it by orders of magnitude (CJK Unified Ideographs is 20,992; Hangul Syllables 11,172), so those
    are paged 500 at a time with Previous/Next. Paging is the only place a cap applies; a normal
    block loads whole.
  - Selecting a font, or opening the view, never loads more than one page of outlines.
  - **Block table:** a static table in core generated from the Unicode `Blocks.txt` (a few hundred
    entries), so no new dependency. `unicode-script` is already in the tree through egui, but it
    maps characters to scripts, not blocks, and scripts do not give the "code page" grouping you
    described.
- **Where the cost is real, say so.** Enumerating a font's code points (the cmap) is cheap; loading
  outlines is what costs. The picker never loads outlines.

This requires reloading the font when the text gains a new non-Latin-1 character. Face parsing from
a memory-mapped blob is cheap, so reloading per edit is acceptable; if profiling disagrees, the
loaded font can cache outlines per code point.

## Default font (Q5)

Source of the current built-ins: the `epaint_default_fonts` crate that egui uses. It is licensed
`(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`: Ubuntu Light is under the Ubuntu Font License
1.0, which is free but has renaming conditions for modified versions and is less permissive than the
rest; Hack is under an MIT-style license (with the Bitstream Vera terms for its base glyphs). The
GUI also already bundles Lato (OFL-1.1) and Lucide (ISC) for its own interface.

Recommendation: **bundle no engraving font in `rengrave_core`.** The default becomes the system's
generic sans-serif, resolved through `fontique` (Helvetica on macOS, Segoe UI/Arial on Windows, the
fontconfig default on Linux). Consequences to accept:

- Output for the same job differs between machines unless the font is pinned. That is inherent to
  "use system fonts"; the job records the family so it is at least visible.
- A headless machine with no fonts (some containers) fails with an explicit error telling the user
  to install a font or pass `--font-file`. The old zero-setup claim in README and DECISIONS D5
  changes accordingly.
- Tests and docs use a small fixture font checked into `tests/`. Candidate: Hack (MIT-style, already
  in the dependency tree). The `epaint_default_fonts` dependency moves out of core and remains only
  where the GUI needs it for its own interface.

If you later want a guaranteed fallback, the lowest-friction single font is Hack, because it is
already in the GUI binary and its license is MIT-style; it is monospace, which is a poor general
default, so I would not make it the default selection.

## Status

Font Page mode is implemented: `InputType::FontPage`, the Unicode block chooser (only blocks the
font covers, with counts), 500-code-point pages, and `--font-page`/`--page` on the CLI. The block
and page are saved with the job as `font_block` and `font_page`. `show_font_index` is gone.

## Options

### Option A: Basic

System fonts and a minimal user library.

- `FontRef`, `FontCatalog`, `system-fonts` feature, background scan.
- System families load their regular face. TTC face index supported in `from_outline_font`.
- Latin-1 plus text characters loading, and block-by-block "Show all characters" (Large fonts, above).
- `AppSettings` with `serde` + `serde_norway`.
- Application settings dialog with a Fonts page: add/remove user font files; library persisted.
- Picker with "User fonts" and "System fonts" groups and the filter box.
- Remove `font_dir`, the folder chooser, `Font::list_files`, and the built-in fonts from core.
- CLI: `--font system:Name`, `--font-file`, and a `fonts` command listing both libraries.
- Not included: choosing bold/italic/other faces, variable-font weight, missing-font fallback.

Effort: about 1 to 1.5 weeks (the settings store and dialog are new work, not just font code).
Risk: low to medium. Weakness: a family such as Futura has Medium, Bold, and Condensed faces the
user cannot reach except by adding the file to the user library.

### Option B: Balanced (recommended)

Everything in A, plus:

- **Face selection:** a second combo lists the family's faces (Regular, Bold, Italic, Condensed...).
  Identity becomes `system:Family;Face`.
- **Variable fonts:** a weight slider when the face has a weight axis, stored in the job.
- **Missing-font handling:** if a job names a font this machine lacks, keep the setting, show a
  status-bar warning, and calculate with the default font so the job still produces a preview.
- **Library hygiene:** drop duplicate family aliases, sort case-insensitively, hide non-outline
  faces, remember recently used fonts, and let user fonts carry an editable display name.
- Docs: README, DECISIONS (new entry superseding D5), PORTING (replaces the folder-scan row).

Effort: about 2 to 3 weeks in total. Risk: medium (variable fonts and alias de-duplication need
real-machine testing on all three platforms).

### Option C: Bells and whistles

Everything in B, plus:

- **Live previews:** each picker row drawn in its own typeface, using a custom egui `FontProvider`
  (`fonts_for_family(FontFamily::Name(..))`) backed by the same `fontique` collection, registered
  lazily for visible rows only so egui's atlas does not bloat.
- **Engraving suitability:** flag faces whose thinnest strokes are narrower than the selected bit
  (V-carve) or whose counters would close at the chosen height. This needs a pass over the flattened
  strokes and is the feature unique to a CNC tool.
- **Per-character fallback:** if the face lacks a character, fill from the fallback face `fontique`
  chooses for that script instead of reporting "Not in font".
- **Favorites and categories:** serif, mono, script via `fontique` generic families.
- **Drag and drop** of a font file onto the window to add it to the user library.

Effort: about 4 to 5 weeks. No new dependencies beyond B. Risk: medium; suitability analysis is the
unknown.

## Comparison

| | A Basic | B Balanced | C Bells and whistles |
|---|---|---|---|
| System fonts and user font library | yes | yes | yes |
| Application settings dialog (Fonts page) | yes | yes | yes |
| Large fonts cheap by default | yes | yes | yes |
| Pick bold/italic/other faces | no | yes | yes |
| Variable-font weight | no | yes | yes |
| Missing-font handling | error | warn + fallback | warn + fallback |
| Row previews in own font | no | no | yes |
| Bit/size suitability hints | no | no | yes |
| Drag and drop to add fonts | no | no | yes |
| Windows/Linux work without new code | yes | yes | yes |
| Effort | 1-1.5 weeks | 2-3 weeks | 4-5 weeks |

## Recommendation

Build **B**, in this order, each its own PR:

1. **Core:** switch the outline reader from `ttf-parser` to `skrifa` first, as its own change with
   the existing font tests (and, before the old code is deleted, a comparison of stroke output from
   both readers on the repo's fixture fonts). Then `FontRef`, `FontCatalog`, load-on-demand glyphs,
   removal of `font_dir` and the built-in fonts, and fixture-based tests.
2. **Application settings store and dialog** with the Fonts page (this is shared infrastructure the
   later machining and tool libraries will also use, so it is worth getting the storage format right).
3. **Picker, Source panel and CLI** changes.
4. **Faces, variable fonts, missing-font handling** (the B additions).

Row previews and suitability hints (C) are good follow-ups once the picker exists.

## Open questions

Decided (Q11): Font Page mode opens on Basic Latin plus Latin-1 Supplement (256 glyphs), and no page
shows more than 500 glyphs; the huge CJK and Hangul blocks get Previous/Next.

Items to verify during implementation: `serde_norway`'s version, maintenance, and license; whether
`skrifa` accepts everything CoreText lists.
