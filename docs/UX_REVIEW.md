# User interface review

This light review of F-Engrave 1.78's interface identifies where its Tk layout works against the
task, and records what `rengrave_gui` does instead. No user testing was involved; these are
judgments from reading the program's source and behavior.

## The workflow

A user of the program works through four questions, in order:

1. What am I cutting? (text and font, or an image)
2. Where does it go on the workpiece? (size, position, origin)
3. How will it be cut? (cut type, bit, feeds, depth)
4. Is it right, and where does the result go? (preview, save)

## Findings and changes

| # | Original behavior | Problem | Change |
| --- | --- | --- | --- |
| 1 | Settings are split between the main window and modal windows: General, V-Carve, and Bitmap. | Modal dialogs hide the preview while you change the settings that affect it. | All settings sit in one side panel with **Design**, **Toolpath**, and **Output** tabs, following the workflow. The preview is always visible. |
| 2 | The user presses **Recalculate** after edits, and the fields and the status bar turn yellow until they do. | Stale results are easy to save, and the extra step slows experimentation. | Layout updates automatically. V-carving runs after a short pause, on a background thread, and restarts if settings change. **Live V-carve** can be turned off for very large designs. |
| 3 | A modal progress window blocks the application during V-carve calculation. | No other work is possible, and a slow job means waiting. | The application stays responsive, with progress and a **Stop** button (`Esc`) in the status bar. The outline appears immediately, before the toolpath. |
| 4 | The save command is always available. If V-carve paths have not been calculated, a message box says so after the attempt. | The problem appears only after the user has asked to save. | **Save G-code** is enabled only when toolpaths match the settings. Its tooltip gives the reason when it is disabled. |
| 5 | Invalid values color the entry field and put one message at a time in the status bar. | The reason is easily missed, and only the most recent problem is visible. | An invalid field shows a red label and its own message directly beneath it. The status bar counts the problems, and calculation pauses until they are fixed. |
| 6 | Numbers are free text in entries, with units in a separate label. | Typing is the only way to change a value, and it is easy to lose track of units. | Numbers can be dragged or typed, and show their unit inside the field. Switching units converts every value. |
| 7 | The origin is a drop-down of 11 names, such as `Bot-Center`. | The list has to be read, and it does not show the geometric relationship. | A 3-by-3 grid mirrors the design's bounding box, with `Design origin` and `Circle center` as named alternatives. |
| 8 | Bit parameters are numbers, with a static picture. | Interactions between angle, diameter, and depth limit are hard to judge. | A live cross-section of the bit follows the settings, and the deepest cut is displayed. Controls not applicable to the selected bit are hidden. |
| 9 | Controls unrelated to the current mode stay visible, often disabled. | The panel is dense, and what matters is hard to find. | Sections appear only when relevant: for example, engrave depth for engraving and bit settings for V-carving. Rarely used sections start collapsed. |
| 10 | The canvas zooms with a left-drag, and with a wheel only where it arrives as X11 buttons 4 and 5. There is no fit command. | Zooming differs across platforms, and getting back to the whole design is awkward. | Scroll or pinch zooms about the pointer, drag pans, and double-click or **Fit** returns. A labeled grid shows scale, and the status bar shows the coordinates under the pointer. |
| 11 | Preview layer options are in the View menu. | Checking the area against the path takes several clicks. | **Area**, **Path**, and **Grid** toggles sit above the preview. The complete list remains in the View menu. |
| 12 | Fonts are listed from one directory, with a separate dialog to change it. | The program does nothing until fonts are installed. | Built-in fonts work immediately, the list has a filter, and any file can be dropped on the window. |
| 13 | G-code can only be seen after saving it or copying it elsewhere. | Checking the output takes a round trip through another program. | A **G-code** view next to the preview shows the program with syntax highlighting, updated with the settings. Rapid moves, cutting moves, and depths are told apart at a glance. |
| 14 | The light-gray Tk layout has a fixed size for many controls. | It scales poorly and does not follow system themes. | The panel is resizable, the preview fills the window, and light and dark themes are both supported. |

## Added after the review: Simulate

The **Simulate** tab sits beside Preview and G-code. It plays the G-code program and cuts into a
shaded model of the stock, with play, restart, finish, speed, and a scrub bar that moves in either
direction. It shares its pan and zoom with Preview, shows the current G-code line, and flags rapid
moves that cut into the stock. Choosing the speed from the program's length keeps long jobs
watchable.

## What stays the same

- The terminology of the original's manual (Finish stock, Step over, Drive to corner, and so on),
  so existing users and documentation still apply. Tooltips largely reuse the original's text.
- Every setting remains available, and settings files remain compatible.
- The shortcuts `F5`, `Ctrl`+`S`, `Ctrl`+`G`, `Page Up`, and `Page Down`.

## Not done

Candidates for later, all judged beyond a light review:

- A 3D view of the simulated result
- A library of named bits
- Machining time estimates that account for multi-pass and rapid moves
- Undo and redo
