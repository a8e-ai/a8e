class A8e < Formula
  desc "Articulate (a8e): The sovereign AI operator for your terminal"
  homepage "https://github.com/a8e-ai/a8e"
  license "Apache-2.0"
  version "${VERSION}"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/a8e-ai/a8e/releases/download/v${VERSION}/a8e-aarch64-apple-darwin.tar.bz2"
      sha256 "${SHA256_MACOS_ARM64}"
    else
      url "https://github.com/a8e-ai/a8e/releases/download/v${VERSION}/a8e-x86_64-apple-darwin.tar.bz2"
      sha256 "${SHA256_MACOS_X86_64}"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/a8e-ai/a8e/releases/download/v${VERSION}/a8e-aarch64-unknown-linux-gnu.tar.bz2"
      sha256 "${SHA256_LINUX_ARM64}"
    else
      url "https://github.com/a8e-ai/a8e/releases/download/v${VERSION}/a8e-x86_64-unknown-linux-gnu.tar.bz2"
      sha256 "${SHA256_LINUX_X86_64}"
    end
  end

  def install
    bin.install "a8e"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/a8e --version")
  end
end
