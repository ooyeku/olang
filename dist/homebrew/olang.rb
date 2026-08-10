# Homebrew formula template for olang. Lives in the olang repo as the
# canonical copy; the published version goes in the ooyeku/homebrew-olang
# tap repo (see README.md next to this file).
#
# Per release, update `version` and the three sha256 values — the URLs
# derive from `version` and never change shape. The checksums come
# straight from the SHA256SUMS file that the release workflow attaches
# to every GitHub Release.
class Olang < Formula
  desc "Minimal, expressive language with first-class functions, pipelines, and pattern matching"
  homepage "https://github.com/ooyeku/olang"
  license "MIT"
  version "0.45.0" # release tag without the leading v

  on_macos do
    on_arm do
      url "https://github.com/ooyeku/olang/releases/download/v#{version}/olang-#{version}-macos-arm64.tar.gz"
      sha256 "REPLACE_WITH_MACOS_ARM64_SHA256"
    end
    on_intel do
      url "https://github.com/ooyeku/olang/releases/download/v#{version}/olang-#{version}-macos-x64.tar.gz"
      sha256 "REPLACE_WITH_MACOS_X64_SHA256"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/ooyeku/olang/releases/download/v#{version}/olang-#{version}-linux-x64.tar.gz"
      sha256 "REPLACE_WITH_LINUX_X64_SHA256"
    end
  end

  def install
    bin.install "olang", "otc"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/olang --version")
    assert_match version.to_s, shell_output("#{bin}/otc --version")
  end
end
