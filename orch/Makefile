build:
	cargo build --release

install: build
	ln -sf $(CURDIR)/target/release/orch ~/bin/orch

# IMPORTANT: Do NOT use `cp` to install — macOS kills copied binaries
# due to com.apple.provenance xattr from Claude's sandbox.
# The symlink approach works because macOS trusts the original build output.

test:
	cargo test

clean:
	cargo clean
