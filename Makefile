UMPIRE_SPEC_VERSION := v1.1.0
UMPIRE_SPEC_SHA256  := 11d3f3465e91b4033e62e4a5da08845c76726ef05b43662c59b88ae647d55aa4

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
