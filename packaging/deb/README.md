# The Debian package

Built by the release workflow on the same runners that build the Linux archives, from the
same binary — `dpkg-deb` on ubuntu-22.04, so the glibc floor of the package is the floor of
the archive beside it. The pieces here are the parts that are not derived at build time:

- `control.in` — the control file, with `@VERSION@`, `@ARCH@`, `@DEPENDS@` and
  `@INSTALLED_SIZE@` filled in by the workflow. The Depends line is never edited by hand: it
  is read off the built binary with `ldd` and resolved to owning packages with `dpkg -S`, so
  it states what the binary links, not what somebody remembered it linking.
- `clee.desktop` — the desktop entry, `Terminal=true` because clee *is* a terminal program.

The man page comes from `docs/clee.1` (gzipped into the package), the copyright file from the
repository's `LICENSE`.

What validates the package: the workflow prints `dpkg-deb --info` and `--contents` of the
built package into the job log — the round trip through dpkg-deb's own reader — and the
tag↔`Cargo.toml` guard that gates every artifact gates this one too. To attach the .deb to a
release that is already out without touching the published archives and their checksums,
dispatch the Release workflow with the tag and `only: linux-x86_64-deb` (or
`linux-arm64-deb`).
