# Common tasks for wavelength. Run `make` or `make help` to list them.

FIXTURES := $(wildcard fixtures/*.html)
REPORT   := report
BIN      := target/release/wavelength
VERSION  := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

# The browser window needs a display, so it is only built into the binary on macOS.
ifeq ($(shell uname),Darwin)
FEATURES := --features window
endif

# The macOS app bundle. Override APP_DIR to install somewhere else,
# for example `make build APP_DIR=~/Applications`.
APP_NAME := Wavelength.app
APP      := target/$(APP_NAME)
APP_DIR  := /Applications
ICONSET  := target/icon.iconset

.DEFAULT_GOAL := help
.PHONY: help build binary browse app install uninstall icon debug test parity compare report render fmt lint check clean

help: ## List the available targets
	@awk 'BEGIN {FS = ":.*## "} /^[a-z-]+:.*## / {printf "  %-10s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

ifeq ($(shell uname),Darwin)
build: install ## Build the optimised binary and, on macOS, install Wavelength.app
else
build: binary
endif

binary: ## Build only the optimised binary at target/release/wavelength
	cargo build --release --locked $(FEATURES)

app: binary ## Assemble the macOS app bundle at target/Wavelength.app
	rm -rf target/$(APP_NAME)
	mkdir -p $(APP)/Contents/MacOS $(APP)/Contents/Resources
	sed 's/__VERSION__/$(VERSION)/g' macos/Info.plist > $(APP)/Contents/Info.plist
	cp macos/launcher $(BIN) $(APP)/Contents/MacOS/
	cp macos/icon.icns $(APP)/Contents/Resources/icon.icns

install: app ## Copy Wavelength.app into APP_DIR (default: /Applications)
	@test -d "$(APP_DIR)" || { echo "APP_DIR does not exist: $(APP_DIR)"; exit 1; }
	@# macOS warns when a terminal changes an app that has already been opened,
	@# so the installed copy is only touched when the build differs from it.
	@if diff -rq $(APP) "$(APP_DIR)/$(APP_NAME)" >/dev/null 2>&1; then \
		echo "$(APP_DIR)/$(APP_NAME) is already up to date"; \
	else \
		ditto $(APP) "$(APP_DIR)/$(APP_NAME)" && echo "installed $(APP_DIR)/$(APP_NAME)"; \
	fi

uninstall: ## Remove Wavelength.app from APP_DIR
	@test -n "$(APP_DIR)" || { echo "APP_DIR is empty"; exit 1; }
	rm -rf "$(APP_DIR)/$(APP_NAME)"

icon: ## Regenerate macos/icon.icns from the SVG (needs rsvg-convert and iconutil)
	mkdir -p $(ICONSET)
	for size in 16 32 128 256 512; do \
		rsvg-convert -w $$size -h $$size macos/app-icon.svg -o $(ICONSET)/icon_$${size}x$${size}.png; \
		rsvg-convert -w $$((size * 2)) -h $$((size * 2)) macos/app-icon.svg -o $(ICONSET)/icon_$${size}x$${size}@2x.png; \
	done
	iconutil -c icns $(ICONSET) -o macos/icon.icns

browse: binary ## Open the browser window: make browse [URL="google.com example.com"]
	$(BIN) browse $(URL)

debug: ## Build an unoptimised binary, which compiles faster
	cargo build --locked $(FEATURES)

test: ## Run the unit tests and the doc example
	cargo test --locked $(FEATURES)

parity: ## Run every test, including the one that checks fixtures against Chrome
	cargo test --locked $(FEATURES) -- --include-ignored

compare: binary ## Compare the fixtures against Chrome and write the report
	$(BIN) compare $(FIXTURES) --out $(REPORT)

report: compare ## Run the comparison, then open the report (macOS)
	open $(REPORT)/index.html

render: binary ## Render one page: make render PAGE=fixtures/blocks.html [OUT=render.png]
	@test -n "$(PAGE)" || { echo "usage: make render PAGE=path/to/page.html [OUT=render.png]"; exit 1; }
	$(BIN) render $(PAGE) --out $(or $(OUT),render.png)

fmt: ## Format the code
	cargo fmt

lint: ## Check formatting and run clippy with warnings as errors
	cargo fmt --check
	cargo clippy --locked --all-targets $(FEATURES) -- -D warnings

check: lint parity ## Everything CI runs, plus the Chrome parity test

clean: ## Remove build output and generated reports
	cargo clean
	rm -rf report render.png
