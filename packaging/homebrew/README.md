# Homebrew tap

`clee.rb` is the formula. Homebrew serves formulae from a repository named
`homebrew-<tap>`, so it has to live in its own repo — `homebrew-clee` — not in this one.
The copy here is the source of truth to copy from when cutting a release.

## One-time: create the tap

```bash
gh repo create msavox/homebrew-clee --public --description "Homebrew tap for CleeCode (clee)"
git clone https://github.com/msavox/homebrew-clee.git
mkdir -p homebrew-clee/Formula
cp packaging/homebrew/clee.rb homebrew-clee/Formula/clee.rb
```

Then fill in `url` and `sha256` (see below), commit and push.

## Each release

1. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`. The Release workflow builds
   the macOS binaries, attaches them to the GitHub release, and prints the formula's `url`
   and `sha256` in its run summary (job "Homebrew source checksum").
2. Copy those two lines into `Formula/clee.rb` in the tap, then commit and push.

To compute the checksum by hand instead:

```bash
curl -fsSL https://github.com/msavox/cleecode/archive/refs/tags/v0.1.0.tar.gz | shasum -a 256
```

## Verifying the formula before publishing

```bash
brew install --build-from-source ./Formula/clee.rb
brew test clee
brew audit --strict --formula ./Formula/clee.rb
```

## Installing, once the tap exists

```bash
brew install msavox/clee/clee
```

## Notes

- The formula builds from source (`depends_on "rust" => :build`), which keeps one recipe
  working on both Apple Silicon and Intel with no bottles to maintain. The prebuilt
  tarballs attached to each release are for people who would rather not build.
- `homebrew-core` is a separate step and has notability requirements (a formula generally
  needs a project with real traction); a tap has none, which is why this starts here.
