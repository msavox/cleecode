class Clee < Formula
  desc "Terminal IDE with file-tree sidebar, embedded terminals and syntax highlighting"
  homepage "https://github.com/msavox/cleecode"
  # Both values are printed in the release workflow's run summary ("Homebrew source
  # checksum"), so bumping a version is a copy/paste.
  url "https://github.com/msavox/cleecode/archive/refs/tags/v0.1.2.tar.gz"
  sha256 "REPLACE_WITH_THE_SHA256_FROM_THE_RELEASE_RUN_SUMMARY"
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
  end

  def caveats
    <<~EOS
      The file-tree icons need a Nerd Font. To install the bundled one for your user:
        clee --install-font

      A copy also lives at:
        #{opt_pkgshare}/fonts
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/clee --version")
    assert_match "USAGE", shell_output("#{bin}/clee --help")
  end
end
