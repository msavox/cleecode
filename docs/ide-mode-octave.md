# CleeCode + Octave: workspace panel and docked plots

Prepared 2026-08-20 in a separate session, on Octave 11.3.0 (Homebrew, macOS arm64).
Everything below marked *measured* was run against a real interactive Octave in a PTY,
not reasoned about. The `.m` files in `assets/octave/` are working code, not sketches.

Repo state this was written against: `master` @ `1c6a6f5` (0.6.2).

---

## 1. Why not the obvious approach

The tempting way to read the workspace is to type `whos` into the Octave prompt CleeCode
already knows how to write to (`app.rs:3661` does exactly this for `run(...)`). Don't: it
pollutes the user's transcript, it fights whatever they are half-way through typing, and it
does nothing while the interpreter is busy.

## 2. The mechanism

`add_input_event_hook(FCN, DATA)` — Octave calls FCN *while it waits for input at the
prompt*. This is the same idle moment the Octave GUI refreshes its own workspace dock from.

Measured:

- fires every **~105 ms** while genuinely idle at the prompt;
- **does not fire at all** while a command runs, so a busy interpreter is never disturbed;
- `evalin("base", ...)` from inside the hook sees the live base workspace;
- prints **nothing** — the user's transcript is untouched;
- bursts **coalesce** rather than get lost: several commands arriving back-to-back produce
  one snapshot, taken after the last one, and it is correct.

Limitation: interactive sessions only. `octave script.m` never waits at a prompt, so it
never produces a snapshot. Fine for a live panel; worth knowing before promising more.

## 3. Change detection — the part that is easy to get wrong

Three traps, all hit and fixed during development:

**Timing alone does not work.** The first attempt treated "hook silent for >0.3 s" as "a
command just finished". It misses every fast command: `a = 5;` completes inside one tick.

**The reliable trigger is `numel(history())`** — it grows by one for *every* command
entered. It catches instant commands, and it catches `a(1) = 99;`, where name, class, size
and bytes are all unchanged and only the value moved. The last history line is compared as
well, because once `history_size` (default 1000) saturates the count stops growing.

**Building the fingerprint naively costs more than everything else combined.** Growing a
string in a loop is O(n²): *measured 7.46 ms per tick* at 208 variables — 7.5% of a core,
burned forever, just sitting at the prompt. One vectorised `sprintf` per field, letting the
struct array's cs-list expand into it, gives **0.54 ms** — 14× better, same workspace,
including a 32 MB matrix. (`whos` itself is only ~0.2 ms: it reads metadata, not data.)

Verified with 11 commands sent one at a time: assignment, in-place edit, complex (`attr:"c"`,
stats over `|z|`), NaN counted and excluded from stats, `clear`, struct, `global`
(`attr:"g"`), and **two identical `a = a + 1;` in a row** — the worst case, since history's
last line and every metadatum match. 11/11.

## 4. JSON contract

Written to `$CLEECODE_OCTAVE_WS`, write-to-temp + `rename`, so a reader never sees half a
file. Watch mtime on the redraw tick; no fs watcher needed.

```json
{
  "v": 1,
  "seq": 12,
  "time": 1787214595.164,
  "pid": 65454,
  "cwd": "/Users/matteosavoia/proj",
  "vars": [
    {"name":"a","class":"double","size":[1,10],"bytes":80,"attr":"",
     "min":1,"max":1001,"mean":500.5,"nans":0,"preview":"[999;2;3;4;5;6;7;8]"}
  ]
}
```

- `attr` uses the same letters `whos` prints: `c` complex, `s` sparse, `g` global,
  `p` persistent.
- `min`/`max`/`mean` are `null` where they make no sense (char, cell, struct, handle) and
  where the array exceeds `STAT_LIMIT` (1e6 elements), in which case `preview` says
  `"(too large to summarise)"`. NaN and Inf serialise as `null` — plan the Rust type as
  `Option<f64>`.
- `pid` is the Octave PID. Compare it against what `dnd::shell_running_octave` already
  computes to bind a snapshot to the right pane and to spot a stale file.

**Gotcha worth keeping:** `vars` is built as a *cell array*, not a struct array, because
`jsonencode` turns a 1×1 struct array into a bare object rather than a one-element array —
a workspace holding exactly one variable would otherwise serialise to a different shape than
every other. A cell array is always an array, empty included. Don't "simplify" this back.

## 5. Installing the hook

The user may start Octave by typing `octave` themselves, so CleeCode cannot pass flags.
Env-gated block in `~/.octaverc`, inert everywhere else:

```matlab
## --- CleeCode: workspace panel ---
if (! isempty (getenv ("CLEECODE_OCTAVE_WS")))
  addpath (getenv ("CLEECODE_OCTAVE_LIB"));
  cleecode_ws (getenv ("CLEECODE_OCTAVE_WS"));
endif
## --- end CleeCode ---
```

*Measured*: inside CleeCode's env the hook installs itself and the panel data appears with
the user having typed nothing but `octave`; in a normal terminal the block produces no
output and no error.

The two env vars go next to the `cmd.env("CLEECODE", "1")` that **already exists** at
`src/terminal_panel.rs:365`. Give each pane its **own** snapshot path, so several Octave
prompts each drive their own panel.

## 6. Docked plots — feasible, with one honest caveat

Reparenting a live Qt figure window into a TUI is not possible. What *is* possible is a
raster hand-off, and CleeCode already has every piece:

- `set(0, "defaultfigurevisible", "off")` — no OS window ever appears;
- the **same idle hook** prints each figure to PNG (*measured*: works from inside the hook);
- `preview.rs` already draws PNGs through `ratatui_image` (Kitty / iTerm2 / Sixel).

*Measured* render cost: **~813 ms for the first print** (Qt/OpenGL init, paid once) and
**~46 ms to re-print after `xlim([20 60])`**.

That 46 ms is the interesting number: navigation can be *rebuilt* rather than inherited.

### A plot opens as an editor tab, like a PDF

This is the shape to build (Matteo's call, and the right one): a figure becomes a preview
tab with its own nav bar, exactly like PDF / Markdown / PNG. `Preview` already carries
`zoom`, `fit`, `scroll_px`, `scroll_x` and `pages`, so the chrome is there.

**But a plot tab should not zoom the way a PDF tab zooms**, and this is the whole point.
PDF zoom is raster zoom: rasterise bigger, scroll around. For a figure you can instead push
the zoom *back into Octave* as `xlim`/`ylim` (or `view` for 3-D) and re-print — CleeCode
already knows how to write to that prompt (`app.rs:3661`). The difference is not cosmetic:

- raster zoom magnifies pixels, and the axis labels stay wrong — they still describe the
  original range;
- axis zoom re-renders at 46 ms, is sharp at any depth, and **the ticks relabel themselves**,
  which is what makes it feel like the real figure window.

Same argument for pane resize: don't scale a fixed PNG, re-print at the pane's actual pixel
size (`figure position` + `-r`). Octave is a rasteriser that can be asked for the exact size
wanted — poppler, for PDFs, is used the same way.

This makes the plot tab the first **bidirectional** preview in CleeCode: every other tab
only reads a file. Worth designing for deliberately rather than discovering later.

Fallback, and it matters: if the Octave session that produced the figure is gone, the round
trip is impossible. Fall back to raster zoom over the last PNG — a stale plot is still a
picture, and it should not become an error tab. The `pid` in the snapshot JSON (§4) is how
the tab knows which session it belongs to and whether it is still alive.

### The nav bar must match the Octave figure window

Required minimum: **pan, rotate, zoom in, zoom out**. All four are reproducible on an
*invisible* figure — this was the risk, since `zoom`/`pan`/`rotate3d` exist in Octave
primarily as mouse modes that want a window. *Measured*, all on `visible="off"`:

| action    | command sent to the prompt        | result                    | cost |
|-----------|-----------------------------------|---------------------------|------|
| zoom in   | `zoom(2)`                         | xlim `[0 100]` → `[25 75]` | 1 ms |
| zoom out  | `zoom(0.5)`                       | back to `[0 100]`          | 1 ms |
| pan       | `xlim(xl + 0.25*diff(xl))`        | `[25 125]`                 | <1 ms |
| rotate 3-D| `view(45, 30)`                    | `view()` → `[45 30]`       | — |

Plus the re-print: **32 ms** for a 2-D line plot, **60 ms** for `surf(peaks(30))` including
the `view` change. So a full nav step costs ~30–60 ms end to end: fine for keyboard nav, and
still 16–30 fps if a mouse drag is mapped onto it.

Use the programmatic `zoom(factor)` form, not the `zoom on` mouse mode — the mode needs a
real window, the factor form does not.

### Mouse

CleeCode already has the drag framework — `DragTarget` in `app.rs:668` with press/drag/release
state, plus scroll — so plot navigation is two new variants (`PlotPan`, `PlotZoomRect`) rather
than new machinery. With the mouse the nav can match the GUI properly: drag to pan, wheel to
zoom, rubber-band rectangle to zoom to a region, drag to rotate in 3-D.

That needs one thing the workspace snapshot does not carry: **a geometry sidecar per figure**,
so a pane pixel can be turned into a data coordinate without a round trip per mouse move.

```json
{"fig":1, "png":[800,600],
 "axes":{"pos":[0.13,0.11,0.775,0.815], "xlim":[0,100], "ylim":[-1,1],
         "xscale":"linear", "yscale":"linear", "is3d":false, "view":[-37.5,30]}}
```

`pos` is the axes rectangle normalised to the figure, so the axes box in PNG pixels is
`pos .* [W H W H]` — *measured* `x=104 y=66 w=620 h=489` for an 800×600 render. Mind that
`pos` has its origin **bottom-left** while a terminal pane counts rows from the top. Carry
`xscale`/`yscale`: with a log axis the mapping is through `log10`, and a linear interpolation
would be quietly wrong rather than visibly broken.

**Sizing trap, and it would have silently misaligned every click.** A figure created with
`position [0 0 800 600]` does *not* print to an 800×600 PNG — *measured*, `print -dpng -r96`
gave **709×532**, because `print` sizes from `paperposition` in inches and ignores the on-screen
pixel size. Force it:

> *Re-measured in this repo on 2026-08-20: the trap is real and the fix below is right, but do
> not carry the pixel pair around as a constant. At `-r100` the same figure gives 739×554. The
> two agree — 709/96 = 7.385 and 739/100 = 7.39, the same ~7.39×5.54 inch sheet — because what
> is fixed is the paper, and the pixels are inches times DPI.*

```matlab
set (f, "paperunits", "inches", "paperposition", [0 0 W/dpi H/dpi], "papersize", [W/dpi H/dpi]);
print (f, "-dpng", sprintf ("-r%d", dpi), file);
```

*Measured*: exactly 800×600 out, 276 ms. This is also how the figure gets rendered at the
pane's true pixel size on resize.

One subtlety on wheel zoom: `zoom(factor)` scales about the axes **centre**. Zooming about the
cursor — which is what feels right — means computing the new `xlim`/`ylim` in CleeCode from the
cursor's data coordinate and sending those instead. The sidecar has everything needed for it.

Two things Octave itself does **not** have, so "exactly like the GUI" does not include them:
`datacursormode` and `clipboard` are both absent in 11.3 (checked with `exist`). If data tips
are wanted later they would be a CleeCode invention, not a port — feasible, since the line
data is one `get(h,"XData")` away, but it is new design, not parity.

**Binary matters, and it is counter-intuitive.** On this Mac:

| binary       | `available_graphics_toolkits()` | offscreen `print` |
|--------------|--------------------------------|-------------------|
| `octave-cli` | `fltk` only                    | **fails** — fltk demands a visible figure and `DISPLAY`, and XQuartz is not installed |
| `octave`     | `qt`                           | works, no window shown |

So a plot-capable Octave workspace must launch `octave --no-gui`, not `octave-cli`.
gnuplot is *not* installed here and is not the default toolkit — worth stating plainly,
since the assumption that Octave plots go through gnuplot is easy to carry over from older
setups.

Not yet done: figures are re-printed on every tick in the probe. Reuse the section-3
command-boundary trigger so each figure is printed once per command instead.

## 7. Proposed built-in `octave` workspace

`clee -w octave` should open something that reads like the Octave IDE. The plumbing already
exists: `workspace::default_workspace(root)` is a built-in, `is_default()` gates it, and
`WorkspaceTab { name, startup_command }` is exactly the right shape.

Sketch:

- terminal 1, tab `octave` — `startup_command = "octave --no-gui"` (see the table above);
- terminal 2, tab `workspace` — a viewer tailing the snapshot JSON.

The viewer-in-a-terminal is the cheap route: no new panel type, and the Workspace TOML
already carries it, so `clee -w octave` can ship before any panel work. The trade-off is
that a terminal cannot be docked, resized or focused like a real panel — so if the workspace
view ends up wanting the same treatment as plots (its own tab, its own chrome), the JSON
contract in §4 does not change. It was kept independent of the presentation on purpose.

Between them, the two halves give the Octave IDE layout: prompt, workspace, and figures as
tabs — which is what `clee -w octave` is for.

## 8. Files

```
  assets/octave/cleecode_ws.m        install / remove the hook
  assets/octave/cleecode_ws_tick.m   the hook: change detection + JSON snapshot
  assets/octave/wsinfo.m             standalone `whos` with min/max, useful on its own
  scripts/ide/octave_ws_test.py       11 commands, one at a time — the real regression test
  scripts/ide/octave_ws_e2e.py          burst behaviour and transcript cleanliness
  scripts/ide/octave_plot_probe.m         figure-to-PNG probe from inside the hook
```

`octave_ws_test.py` is the one to keep: run it from the directory holding the `.m` files.

## 9. Watch out when testing

The PTY harness must keep draining the master fd. An early version stopped reading between
sends, Octave blocked, and it looked exactly like a lost update — it was the harness, not
the hook. If snapshots seem to go missing, suspect the test first.
