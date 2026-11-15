.SHELLFLAGS := -ec
UMPIRE_SPEC_VERSION := v1.2.0
UMPIRE_SPEC_SHA256  := b88f0edabd6c5dde6d9faa863e1e4bf8c0457d0c6e691a9819a69278a6ae2dce

spec/.synced-at-version: Makefile
	@tarball=$$(mktemp -t umpire-spec-XXXXXX.tar.gz); \
	curl -fsSL "https://github.com/umpire-tools/umpire-spec/archive/refs/tags/$(UMPIRE_SPEC_VERSION).tar.gz" -o "$$tarball"; \
	echo "$(UMPIRE_SPEC_SHA256)  $$tarball" | sha256sum -c -; \
	rm -rf spec; mkdir -p spec; \
	tar -xzf "$$tarball" -C spec --strip-components=1; \
	echo "$(UMPIRE_SPEC_VERSION)" > spec/.synced-at-version; \
	rm "$$tarball"

.PHONY: test
test: spec/.synced-at-version
	cargo test

.PHONY: spec-sync
spec-sync: spec/.synced-at-version

.PHONY: clean
clean:
	rm -rf spec target
