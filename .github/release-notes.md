## What's new in 0.3.0

### It no longer closes on you

A panic used to take the whole editor down, and with it every shell running inside — an ssh
session, a long-running build, a `claude` in a pane, gone with no way back. Three real crashes
are fixed at the source (a terminal opened at zero height, a split editor in a very narrow
window, a stale tab index), and a safety net now contains anything left: a panic is reported in
the status line and written to `panic.log` instead of ending the session. A broken terminal
costs you that terminal, at most.

### Keys you can actually press

The bindings were rebuilt around two facts. Function keys and PageUp/PageDown need Fn on a
laptop, so they are gone — every one of them. And macOS only sends Option as Meta on US keyboard
layouts, so `Alt`+letter never arrived at all on an Italian, German or French keyboard; those are
gone too. What replaces them:

- **`Ctrl+Alt`+arrow** moves to the frame in that direction — sidebar, either half of a split
  editor, or a tiled terminal, whichever is actually there. (`Ctrl`+arrow alone belongs to
  macOS, which uses it for Mission Control and Spaces.)
- **`Ctrl+Shift`+letter** is the application's layer: `M` manual, `B` menu bar, `O` settings,
  `R` run, `T`/`K` terminal tab open/close, `N` new terminal window, `W` save workspace.
  It is safe inside a terminal because no terminal can encode `Ctrl+Shift` for the program
  running in a pane — so nothing there is listening for it.
- **`Ctrl+Shift+←/→`** moves between the tabs of the focused frame; `Ctrl+Tab` cycles the frames.

A focused terminal now gets every other `Ctrl` chord. `Ctrl+J` is Enter to a shell, `Ctrl+E` is
end-of-line, `Ctrl+T` is transpose — the editor no longer eats any of them.

### Named workspaces

Save a whole set-up under a name (`Ctrl+Shift+W`): project root, open files, frame sizes, and the
terminal windows with their tab names and startup commands. Reopening one brings the shells back
already running `claude`, `octave`, `npm run dev`. One hand-editable TOML file per workspace under
`~/.config/cleecode/workspaces/`.

### Also

A built-in manual on `Ctrl+Shift+M`, in English or Italian, syntax-coloured and reachable without
leaving the editor. Save As for buffers that never had a name. And the demo and screenshots are
now generated from scripts in `docs/`, so they cannot quietly go stale again.

## Install

**macOS** (Homebrew):

```bash
brew tap msavox/clee
brew trust msavox/clee
brew install clee
```

All three steps are needed: Homebrew executes a formula's Ruby on your machine, so it refuses
to load one from a third-party tap until you trust the source, and tapping does not imply
trusting.

Or download the archive for your platform from the assets below, unpack it, and put `clee`
somewhere on your `PATH`.

## Platform status

**macOS** is the supported platform: developed, tested and used there daily, on both Apple
Silicon and Intel.

The **Linux** and **Windows** binaries are compiled and started by CI — they build, launch,
and print `--version` — but they have had **no interactive testing at all**. Treat them as
experimental, and please open an issue for whatever breaks. Building from source works too;
see the README for the system dependencies.

The Linux build links against glibc and libxcb: on a minimal or headless system install
`libxcb1` (Debian/Ubuntu) or `libxcb` (Fedora/Arch) if it won't start. Alpine/musl and arm64
aren't covered yet — build from source there.

## First run

The file-tree icons need a Nerd Font. The bundled one installs with:

```bash
clee --install-font
```

Then restart your terminal. `clee --help` lists the rest.
