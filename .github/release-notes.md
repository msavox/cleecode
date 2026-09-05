## What's new in 0.22.0

Four releases in one — the roadmap's 0.19 through 0.22, landed together.

**The agent can point, and asks before it writes.** `clee --mcp` grows from four read-only
tools to seven. `open_file` takes a line range the editor highlights, so "look here" is a
gesture, not a sentence; `preview` renders a file beside your work — Markdown as a document,
pictures and PDFs as pages; `say` puts one line in the status bar, so the agent can narrate
without owning a pane. `open_files` now flags buffers with unsaved edits, and for exactly
those there is `edit_buffer`: the agent proposes a change and the editor asks you — once, for
the whole session, or no — before one character moves. The edit lands as a single undo, in
the buffer, never on disk: saving stays yours. An agent started from the drawer gets all of
it with zero configuration — claude, codex, opencode and gemini each registered by whatever
mechanism that CLI has, in session files that vanish with the editor, never in your own
config. And the lines any outside program writes into your open files now carry a quiet tint
until you take the keyboard back, so what changed while you looked away is findable when you
return.

**Code actions.** The diagnostic under the cursor can fix itself: "add the import", "apply
the compiler's suggestion", offered in a picker and applied with the discipline rename and
format already follow — a diff first where more than one file is touched, one undo step
where one buffer is. rust-analyzer's lazily-resolved actions included.

**Structural selection, and a new keyboard layer.** Widen the selection outward — identifier,
expression, function — and narrow it back, one `textDocument/selectionRange` rung per press.
It lives on `Ctrl+Cmd+↑/↓` (`Ctrl+Super` off macOS): a whole new chord layer, delivered on
terminals speaking the kitty keyboard protocol — Ghostty, kitty, WezTerm, iTerm2, foot — and
absent elsewhere, where the actions stay in the menu and `[keys]` rebinds them. Code folding
now takes its boundaries from the language server where one is attached, falling back to the
brace-and-indent heuristic on dirty buffers and everywhere else.

**A debugger for compiled programs.** C, C++, Rust — anything built with symbols. *Debug ▸
Start debugging* asks what to run with the guess prefilled (Cargo projects guess their own
binary), and runs it under `lldb-dap` or a gdb 14+, found on their own or named in
settings.toml. One set of breakpoints — the same `Ctrl+Shift+P`, the same gutter, any file
now — and stopping marks the line and opens a panel: the stack, the frame's variables, your
watch expressions, the debuggee's output. With the panel focused, `c`, `n`, `s`, `o` and `x`
do what gdb taught your fingers; Continue, Pause, the steps and Stop live in the new Debug
menu too. The whole flow is driven end-to-end in CI against a real adapter — a run that found
and fixed a real bug before release: breakpoints in projects reached through a symlink (every
`/tmp` on a Mac) were accepted, drawn, and never hit.

**The close box wears System 7.** Every ✕ became `□`, moved to the top-left with a cell of
air on each side — `┌ □ ─ Terminal 1` — on windows, tabs and terminal chips alike.

**The turtle is the icon's.** The About drawing and the splash now carry the same top-view
turtle as the app icon, resampled onto the half-block grid in the icon's own greens.

**A chosen theme owns its background.** Every theme now paints its own surface, so switching
themes on a translucent terminal no longer leaves the new theme sitting on the old ground.
Transparency became the explicit choice: *View ▸ Transparent background* (the `●`/`◐` button)
hands the background back to the terminal, and picking a theme takes it whole again.

**Windows installs with two lines.** `scoop bucket add clee
https://github.com/msavox/scoop-clee`, then `scoop install clee` — the Windows twin of the
Homebrew tap.

Also in this release: the status line no longer claims a Markdown document while showing
styled text by request; the interpreter debug keeps its breakpoints to the files it can run
now that the gutter is shared; and both demo recordings on the site and README were reshot —
the six-beat feature reel, and the agent drawer with a real claude editing a real buffer.
