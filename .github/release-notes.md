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
