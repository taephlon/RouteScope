.PHONY: all build install uninstall clean

prefix ?= /usr/local
bindir = $(prefix)/bin

all: build

build:
	@command -v cargo >/dev/null 2>&1 || { \
		echo "Error: Rust/Cargo is not installed."; \
		echo "Please run './install.sh' to automatically install Rust, system build tools, and RouteScope."; \
		exit 1; \
	}
	cargo build --release

install: build
	install -d $(DESTDIR)$(bindir)
	install -m 755 target/release/routescope $(DESTDIR)$(bindir)/routescope
	@if command -v setcap >/dev/null; then \
		echo "Granting raw socket capabilities to routescope..."; \
		setcap cap_net_raw+ep $(DESTDIR)$(bindir)/routescope || echo "Warning: setcap failed. ICMP/TCP modes might require sudo."; \
	fi

uninstall:
	rm -f $(DESTDIR)$(bindir)/routescope

clean:
	cargo clean
