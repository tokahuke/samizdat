class Samizdat < Formula
    desc "Samizdat: your content, available"
    homepage "https://github.com/tokahuke/samizdat"
    version "0.1.0"
    url "https://proxy.hubfederation.com/get-samizdat/$VERSION/node/aarch64-apple-darwin/samizdat.tar.gz"
    revision $REVISION_COUNT
    # sha256 "$SHA256SUM"
    license "AGPLv3"

    depends_on arch: :aarch64

    def install
        bin.install "samizdat"
        bin.install "samizdat-node"
    end

    service do
        run [
            opt_bin/"samizdat-node",
            "--data", "/var/lib/samizdat/node/"
        ]
        log_path "/var/log/samizdat-node-stdout.log"
        error_log_path "/var/log/samizdat-node-stderr.log"
        keep_alive true
        restart_delay 1
    end

    def post_install
        system "samizdat", "hub", "new", "testbed.hubfederation.com", "UseBoth"
    end
end
