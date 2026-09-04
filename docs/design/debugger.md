# The debugger (0.22)

Decided before the code, as the roadmap requires. This document is the choice; the roadmap's
0.22 section is the why-now.

## What exists already, and is kept

CleeCode has been a debugger once before, for Octave: `Ctrl+Shift+P` toggles breakpoints held
in `App::breakpoints` and drawn first in the gutter; the sessions are told about them
(`publish_breakpoints`); the stopped line is highlighted in the editor (ui.rs, "the line the
session is stopped on"); the workspace window shows the *frame's* variables while stopped; and
`scripts/drive_debug.py` proves the loop against a real Octave. 0.22 does not invent a second
debugger beside that — it gives the same shape a second backend, DAP, for compiled programs.

## The adapter, not the protocol zoo

One protocol, DAP, spoken to whichever adapter the machine has:

- macOS: `lldb-dap`, found via `PATH` and then `xcrun -f lldb-dap` (it ships with the Xcode
  command-line toolchain).
- Linux: `lldb-dap` from `PATH`, else `gdb -i=dap` (native since gdb 14 — Ubuntu 24.04 has
  it; Debian 12's gdb 13 does not, and there the LLVM packages provide `lldb-dap`). This is
  the platform that matters most: in this editor's home posture the debuggee, the adapter
  and CleeCode all live on the same machine at the far end of ssh.
- Windows: `gdb -i=dap` from MSYS2 for GNU-toolchain binaries (DWARF — full fidelity), and
  `lldb-dap` from the LLVM installer for MSVC ones, whose PDB reading is honest but partial:
  stepping and variables work, complex types come out rougher than DWARF's. Microsoft's own
  `vsdbg` reads PDB perfectly and is excluded on license, not on merit: its terms bind it to
  VS and VS Code. The manual states the PDB limit and the one-line escape
  (`rustup target add x86_64-pc-windows-gnu`) instead of pretending parity.
- Nothing found: one status line naming what to install for this platform, the way missing
  language servers are reported.

The adapter command is a table entry like `language_servers`, overridable in settings.toml, so
somebody with `codelldb` or a newer gdb points a line at it and nothing else changes.

DAP frames its messages exactly as LSP does — `Content-Length`, then JSON — so the transport
reuses the `lsp.rs` pattern verbatim: spawn, a reader thread, an `mpsc` channel, a `poll_dap`
in the frame loop. No tokio, one concurrency model, as always. The module is `src/dap.rs`,
kept protocol-pure the way `lsp.rs` is: it knows requests, responses and events; it does not
know what a pane is.

## What to debug

The debuggee is an executable, and the editor does not guess silently. *Debug → Start* asks
once per project, with the guess filled in: for a Cargo project, `target/debug/<package>`
(read from `Cargo.toml`); otherwise the last executable the project's run command built, if it
is one; otherwise the picker opens on the project root. The answer is remembered in the
workspace file, beside the other per-project choices. Arguments and working directory live in
the same prompt, remembered the same way.

## Keys: no new chords, and that is the decision

The chord table is full and the Super layer only exists on kitty-protocol terminals — a
debugger that cannot step over ssh in Terminal.app would break the one promise this editor is
built on. So:

- `Ctrl+Shift+P` stays the only global debugger chord: the breakpoint, as today.
- Everything else is a **Debug menu** (start, stop, continue, step over, step in, step out,
  toggle watch) — reachable, palette-indexed, and honest about names.
- While the debug panel has focus, single letters do the work, the way gdb itself spells
  them: `c` continue, `n` step over, `s` step in, `o` step out (`finish` is a menu word, not
  a reflex), `w` add a watch, `x` stop. The panel is where your hands already are when you
  are stepping; a modal layer inside a focused frame breaks no typing anywhere else.

## The panel

The debug panel is a frame in the layout, following the numeric workspace window's precedent:
frames on top (the stack, current frame marked), variables under it (scopes expanded one
level, expandable), watches at the bottom (each `evaluate`d on every stop). Selection with
the arrows, Enter expands or jumps to a frame's site — the jump goes through the same
show-beside-without-focus discipline every other programmatic navigation uses.

Stopping moves the editor to the stopped line with the highlight that already exists;
continuing clears it. Breakpoints hit while the panel is closed open it.

## Waves (implementation order, each wave green before the next)

1. `src/dap.rs`: transport, adapter discovery, session lifecycle (initialize → launch →
   configurationDone), breakpoints, stopped/continued/exited events, stackTrace/scopes/
   variables/evaluate, threads. Unit-tested against a scripted fake adapter, the way lsp.rs
   tests its wire.
2. App integration: start/stop flow with the executable prompt, breakpoint sync (the existing
   map, published to DAP as it is to Octave), stopped-line jump and highlight, continue/step
   plumbing, Debug menu, status lines, i18n.
3. The panel: frames, variables, watches, focus keys.
4. `scripts/drive_dap.py` against the real `lldb-dap` (skipping honestly where absent, like
   drive_debug does without octave), docs (features.md, manual.rs, README), CI wiring.

## What 0.22 does not do

No attach-to-pid, no conditional breakpoints, no data watchpoints, no memory view, and no
debugpy — each is real work with a real audience, and each comes after a release of using
what is here. Written down so their absence reads as a decision, not a gap.
