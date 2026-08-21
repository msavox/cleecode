# Octave and Python, as an IDE

CleeCode does not embed an interpreter. It starts the one you already have, in a terminal you can
type into, and then watches it — so everything below is happening in a plain `octave` or `python3`
session that you could have started yourself, and that keeps working the moment CleeCode is
closed.

That constraint is the reason for most of the design. In particular: **nothing CleeCode wants to
know is typed at your prompt.** Your transcript stays a record of what *you* did, and a busy
interpreter is never interrupted mid-computation to answer a question about itself.

For the keys, the built-in manual (`Ctrl+Shift+M`, section *Octave, Python*) is the reference and
ships inside the binary. This page is the walkthrough.

## Starting

```bash
clee -w octave      # an Octave prompt, already running
clee -w pylab       # a Python one, in the active venv if there is one
```

Each opens one terminal window with two tabs — the interpreter, and a plain shell for `git` and
`pip` — and a second window showing the workspace. A window rather than a tab, because the point
is to glance at it while you work rather than to go and look.

**The panel is not only for those two workspaces.** *Workspace ▸ Show session variables* opens one
in whatever layout you are in, and it watches whichever session last ran something — so it follows
an interpreter you start yourself, in any terminal, however you started it.

Nothing is installed and nothing is written to your home directory. The interpreter is handed the
code and a path to write to through its environment; outside CleeCode, neither does anything. For
Octave that goes through `OCTAVE_PATH` and a `PKG_ADD`, so any `octave` you type in a CleeCode
terminal reports its workspace and hands over its plots, exactly like the preset's does.

On a machine with no display — a remote server over ssh — plots need **gnuplot** installed. It
draws to a file and needs no X and no Qt; CleeCode picks it automatically when there is no
display. With no toolkit at all, the panel says so from the first snapshot rather than letting
`figure()` fail inside your script.

## Running a piece of a file

`Ctrl+Shift+X` sends the selection — or, with nothing selected, the cell the cursor is in — to the
interpreter that is already open. The session keeps its variables, so a script is built up a piece
at a time with the data already loaded, which is the whole reason anyone works this way.

A cell runs from one `%%` line to the next: write it `%%` in Octave and `# %%` in Python, which is
what both worlds already write. A file with no `%%` lines is one cell, so in an undivided script
it runs the script.

```matlab
%% caricamento
t = linspace(0, 4*pi, 400);
ampiezza = 2.5;

%% segnale
segnale = ampiezza * sin(t) .* exp(-t/10);
rumore  = 0.15 * randn(size(t));
misura  = segnale + rumore;
```

![The workspace window filling in from a cell](screenshots/workspace.png)

It goes through a temporary file rather than a paste. A pasted indented block makes Python answer
`IndentationError`, and at an Octave prompt it echoes back line by line into your transcript.

## The workspace window

Every variable the session holds, with its size, class, range and enough of its value to
recognise. It fills in whenever a command finishes — including commands you typed yourself at the
prompt, since it is watching the session and not the editor.

Under the table it lists the last few things you ran, when the pane is tall enough to hold them
without cutting the table short. What CleeCode itself sent is left out: every such line ends in a
marker comment, which is also how your transcript tells you a line was not yours.

If the pane is too short for everything, it says how many rows it is not showing rather than
quietly cutting the list — a panel that shows nine of your twelve variables and looks complete is
worse than one that shows eight and admits it.

## Plots

A plot opens as a tab beside the script that drew it, and no window opens anywhere. A live figure
window cannot be moved into a terminal, so the session is told to open none and hands over a
picture instead. Re-plot and the tab changes with it; a second figure is a second tab. It never
takes the keyboard from what you were writing.

On a figure tab the keys move the plot by **asking the session to draw it again**, not by
magnifying the picture:

| key | |
|---|---|
| `+` / `-` | closer / wider |
| arrows | pan, or turn it if it is a surface |
| `r` | back to the whole plot |
| `e` | write it out as a PDF, in the project folder |

That distinction is the point. Magnified pixels leave the axis labels describing a range that is
no longer on screen — the plot says 0 to 100 while showing 25 to 75 — and no amount of sharpening
fixes a number that is wrong. Redrawing takes about 40 ms, so the honest answer is also the quick
one.

Python plots need **matplotlib installed in the same python your terminal runs**. If figures never
appear, that is almost always why; `pip install matplotlib` in the venv you are using is the fix.

## Looking inside a variable

`Ctrl+Shift+I` offers the session's variables and opens the one you pick: its values, a screenful
at a time, rows and columns numbered. Arrows page around a big one, `Home` returns to the corner,
`R` asks again, `Esc` closes.

A screenful at a time because a 2000×2000 matrix is four million numbers, and nobody wants those
written to disk on the chance somebody looks. The request goes through a file the session's own
idle hook reads, so nothing is typed at your prompt to fetch any of it.

It shows and does not edit. That is a real limit, stated rather than hidden.

## Stopping inside a function

`Ctrl+Shift+P` puts a breakpoint on the cursor's line, or takes it off; the line number goes red.
When the session reaches one it stops, the editor opens that file and marks the line, and the
workspace window shows **the frame's** variables rather than the ones outside it.

![Stopped at a breakpoint](screenshots/debug.png)

That last part is what makes it a debugger rather than a breakpoint setter. In the picture, `a`
and `n` are locals of `calcola` — they are not variables of the session, and while stopped they
are the ones worth reading.

Stopping does not take the keyboard, because you are about to type at the prompt.

**Stepping is typed at the prompt where the session is waiting**: `dbstep` and `dbcont` in Octave,
`n` and `c` at Python's `(Pdb)`. The status line says which. CleeCode offers no key for it, and
that is deliberate in both languages for different reasons — driven from the editor's side Octave's
stepping returns without an error and without moving, and Python's belongs to pdb, where a Python
user already knows to look. A key that quietly does nothing is worse than no key.

Setting the breakpoints, following where you are, and showing what is in scope there is the half
that works from the editor — and it is the half that is awkward by hand.

## Same feature, different machinery

Everything above works in both languages. Almost none of it works the same way underneath, which
matters exactly when something is missing:

| | Octave | Python |
|---|---|---|
| when the session reports | `add_input_event_hook`, ~105 ms idle | an audit hook marks, `sys.ps1` collects |
| recent commands | `history()` | PyREPL's own reader — `readline` reports **none** |
| the frame's variables | `evalin("caller", …)` | `frame.f_locals` |
| breakpoints | `dbstop`, applied from inside the hook | `pdb`, traced for the length of one statement |
| stepping | `dbstep` / `dbcont` | `n` / `c` |
| redrawing a figure | only when it says `__modified__` | only when matplotlib says it is stale |

Tracing for the length of one statement is the choice worth explaining. Python's prompt is not
idle — PyREPL redraws the line on every keystroke — so a trace function left installed there would
pay a call per line of code for every *character typed*, and catch nothing. The audit hook fires
once, immediately before your statement runs, which is the only window in which a breakpoint can
be reached.

## When something is wrong

`CLEECODE_DBG_LOG=/tmp/clee.log` makes both hooks say why they failed. Everything they do runs
inside a `try` that must not break your REPL, which means a mistake in there has exactly one
symptom: a panel that quietly stops changing. That variable turns the silence off.

## How this was built

The [design notes](design/) are the working record: what was measured against a real interpreter,
what the measurements ruled out, and why the two languages ended up sharing one seam.
