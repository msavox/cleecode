class Clee < Formula
  desc "Terminal IDE with file-tree sidebar, embedded terminals and syntax highlighting"
  homepage "https://github.com/msavox/cleecode"
  # Both values are printed in the release workflow's run summary ("Homebrew source
  # checksum"), so bumping a version is a copy/paste.
  #
  # Whatever you take the checksum from, check it is an archive first. A `curl | shasum` while
  # GitHub was returning 429 once put the hash of a 199-byte error page in here, and a formula
  # with the wrong checksum does not fail loudly — it downloads, mismatches, and looks like a
  # hung install.
  url "https://github.com/msavox/cleecode/archive/refs/tags/v0.14.0.tar.gz"
  sha256 "727cf0e9e943fb7853103ed9ecb3d30ced2a89ce569bb7f7fa26f4a03fe09ef8"
  license "MIT"
  head "https://github.com/msavox/cleecode.git", branch: "master"

  depends_on "rust" => :build

  # Homebrew on Linux builds this from source too, and the clipboard integration (arboard)
  # needs libxcb there — on macOS the system frameworks cover it.
  on_linux do
    depends_on "pkgconf" => :build
    depends_on "libxcb"
  end

  def install
    # Builds the [[bin]] named `clee` (the crate itself is `cleecode`).
    system "cargo", "install", *std_cargo_args
    # Keep the bundled Nerd Font with the install, so it stays available even if the
    # source tree is gone.
    pkgshare.install "assets/fonts"
    man1.install "docs/clee.1"
  end

  def caveats
    <<~EOS
      The file-tree icons need a Nerd Font. To install the bundled one for your user:
        clee --install-font

      A copy also lives at:
        #{opt_pkgshare}/fonts

      On macOS, to give CleeCode an icon in the Dock and let Finder open
      files with it (needs Ghostty, and builds the bundle locally so it
      arrives without Gatekeeper's quarantine):
        clee --install-app

      Previews are optional extras, not requirements — without them CleeCode
      simply shows less, rather than failing:
        brew install poppler     PDF pages (ghostscript also works)
        brew install pandoc typst  Markdown as a real document, pictures and all
        brew install chafa       pictures inside a terminal pane

      The numeric workspaces (clee -w octave, clee -w pylab) drive the
      interpreter you already have, and nothing is installed into it.
      Nothing here is bundled either:
        brew install octave      an Octave session to run cells in
        brew install gnuplot     plots from an Octave session with no display
                                 (a remote server over ssh) — it needs no X
        pip install matplotlib   plots from a Python session — it has to be
                                 the same python your terminal runs
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/clee --version")
    assert_match "USAGE", shell_output("#{bin}/clee --help")
  end
end
