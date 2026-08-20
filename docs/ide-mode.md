# One feature, two backends

Read this before `ide-mode-octave.md` (Octave) or `ide-mode-python.md` (Python). Those two describe
the mechanisms; this describes the seam between them.

Every feature in the plan applies to both languages. That is not a coincidence to notice
later — it is the shape of the thing, and building it as two parallel implementations that
"might converge" produces two half-features that never do. **The Rust side should know about
one session abstraction with two adapters from the first commit.**

Ship `clee -w octave` and `clee -w pylab` off the same code.

*(This line said `octavelab` when the document arrived, which contradicted the Naming section
at the bottom — that section argues against `octavelab` at length and is the one that stands.)*

## Everything the Rust side needs from a backend

Each row is verified against a live interpreter, not inferred. The point of the table is how
*short* it is: the whole language-specific surface is fifteen one-liners.

| operation | Octave | Python |
|---|---|---|
| install the hook | `~/.octaverc`, env-gated → `add_input_event_hook` | `PYTHONSTARTUP` env var → `sys.ps1.__str__` |
| snapshot cadence | polls ~10 Hz, needs change detection | once per prompt, free |
| run a file | `run('f.m')` | `exec(open('f.py').read())` |
| run a selection / `%%` cell | temp file + `run(...)` | temp file + `exec(...)` |
| delete a variable | `clear x` | `del x` |
| read a slice (inspector) | `a(1:50, 1:20)` | `arr[0:50, 0:20]` |
| write a cell (inspector) | `a(3,4) = 7` | `arr[3,4] = 7` |
| completion, live namespace | `completion_matches(s)` | `rlcompleter` over `__main__` |
| history | `history()` | `readline.get_history_item` |
| render a figure | `print(f,'-dpng','-rDPI',p)` + the `paperposition` fix | `fig.savefig(p, dpi=...)` |
| zoom | `zoom(2)` or `xlim([a b])` | `ax.set_xlim(a, b)` |
| pan | `xlim(xl + k*diff(xl))` | `ax.set_xlim(...)` |
| rotate 3-D | `view(az, el)` | `ax.view_init(elev, azim)` |
| export a figure | `print(f,'-dpdf'/'-dsvg',p)` | `fig.savefig('x.pdf'/'x.svg')` |
| debugger | `dbstop` / `dbstack` / `dbstep` | `pdb` |
| error location | `err.stack` (file, line, column) | traceback |

Both languages already emit the same snapshot JSON (`ide-mode-octave.md` §4) with a `"lang"` field,
so the panel, the variable inspector and the plot tab are written once.

## The three places the abstraction must NOT paper over

**Snapshot cadence.** Octave polls and needs the whole change-detection apparatus; Python
gets an exact per-statement callback for free. Do not force Python through a polling
interface to "match" — you would be adding cost and latency to buy symmetry nobody sees.

**Live plot updates during a long computation.** Python can, through the custom matplotlib
backend's `draw_idle`. Octave cannot: its hook does not fire while the interpreter is busy.
This is a real capability difference, not an implementation gap. Let the backend advertise
it and let the UI do less where it is absent, rather than promising it in both and quietly
failing in one.

**Which binary is "the interpreter".** For Octave, plotting requires `octave` (qt toolkit),
**not** `octave-cli` (fltk here, and fltk demands an X11 display that is not installed on
this machine). For Python it is whatever the active venv resolves to, which CleeCode already
tracks via `apply_venv`. Two different problems wearing the same name.

## Feature list, in build order

1. **Run selection / `%%` cells from the editor.** The one that makes it an IDE rather than a
   viewer. Same cell convention in both languages. Inject via a temp file, never by pasting a
   multi-line block at the prompt — Python's REPL mangles pasted indented blocks.
2. **Clickable tracebacks** that open the file at the line. Python tracebacks are highly
   regular; Octave's error struct carries `stack` with file, line and column.
3. **Variable inspector as a tab**: a paged grid, editable, writing back through the
   round trip. This is the Octave GUI's variable editor.
4. **History panel.** Nearly free — the Octave adapter already reads `history()` for its
   change detection.
5. **Completion from the live session.** Verified: with `myvar = 42` in memory,
   `completion_matches("myv")` returns `myvar`. CleeCode has `complete.rs` already.
6. **Figure export** (PNG/PDF/SVG), matching the figure window's Save.

Then, as a larger tier on the same channels: a real debugger — breakpoints in the editor
gutter, a navigable stack, and the frame's own workspace in the panel.

## Naming

`clee -w octave` and `clee -w pylab`.

The asymmetry is deliberate. `pylab` exists because *Python* is ambiguous: a Django layout
and a numerical one have nothing in common, so the preset has to say which mode of work it
means. Octave has no such ambiguity — it is already the numerical-computing-with-plots tool,
and there is no other kind of Octave session the name could mislead about. An `octavelab`
would disambiguate nothing, and `clee -w octave` is what anyone types first.

`pylab` is also not a pattern to extend — it is an established term (numpy plus pyplot,
Python used the way MATLAB and Octave are). Coining `octavelab` to rhyme with it imitates a
pattern that is not there. The rule that scales is: **each preset takes the most recognisable
term in its own community** — `octave`, `pylab`, and later `julia`, not `octave-lab`,
`python-lab`, `julia-lab`.

### A bug to fix while adding built-ins, not to name around

Built-in workspace names are reserved and they shadow the user's own **silently**:
`main.rs:286` resolves `if is_default(n) { None } else { load(n) }`, checking the built-in
before ever looking for the file, and `save_in` refuses to overwrite one while reporting a
message that always names `DEFAULT_NAME`.

So taking the name `octave` can hide the workspace of someone who had already saved one
under it. The fix is in the code, not in the name: `is_default` should become "is one of the
built-ins", the save error should name the workspace actually clashed with, and a shadowed
user workspace should say so rather than vanish. Picking awkward preset names to dodge this
would buy a permanently worse name and leave the bug waiting for the next built-in.
