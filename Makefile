# Conveniences for wiring the toolchain into an editor. `make vscode`
# builds the language server, bundles it (and the standard library, when
# the submodule is checked out) into the VSCode extension, packages the
# extension as a .vsix and installs it -- zero configuration afterwards.

CARGO ?= cargo
NPM ?= npm
NPX ?= npx
CODE ?= code

EXT_DIR := editors/vscode
LIBRARY := vendor/sysml-v2-release/sysml.library
SERVER := target/release/sysml-lsp
VSIX := $(EXT_DIR)/sysml-v2.vsix

DEMO := examples/order-system
FIXTURE_JSON := crates/sysml-import-api/tests/fixtures/inventory_store.rustdoc.json

.PHONY: help lsp vscode vscode-package vscode-clean demo

help:
	@echo "make demo            Rust API -> SysML -> model -> Rust -> run, end to end"
	@echo "make vscode          build, package and install the VSCode extension"
	@echo "make vscode-package  build the .vsix without installing it"
	@echo "make lsp             build the language server only"
	@echo "make vscode-clean    remove the extension's build artifacts"

# the whole loop: import the existing API, resolve the model against it,
# generate the calling code, compile and run it against a stub
demo:
	$(CARGO) build --release -p sysml-cli
	target/release/sysml import-rust $(FIXTURE_JSON) -o $(DEMO)/model/InventoryStoreApi.sysml
	target/release/sysml check $(DEMO)/model
	target/release/sysml rustgen $(DEMO)/model/order_system.sysml \
		--library $(DEMO)/model/InventoryStoreApi.sysml \
		--library $(DEMO)/model/scalars.kerml \
		-o $(DEMO)/src/generated.rs
	cd $(DEMO) && $(CARGO) run --quiet

lsp:
	$(CARGO) build --release -p sysml-lsp

$(EXT_DIR)/node_modules: $(EXT_DIR)/package.json
	cd $(EXT_DIR) && $(NPM) install --no-audit --no-fund

# the extension looks for server/sysml-lsp and library/sysml.library next
# to its own files before falling back to settings and PATH
vscode-package: lsp $(EXT_DIR)/node_modules
	mkdir -p $(EXT_DIR)/server
	cp $(SERVER) $(EXT_DIR)/server/
	@if [ -d "$(LIBRARY)" ]; then \
		rm -rf $(EXT_DIR)/library; \
		mkdir -p $(EXT_DIR)/library; \
		cp -R $(LIBRARY) $(EXT_DIR)/library/; \
	else \
		echo "note: $(LIBRARY) not checked out; library names will not resolve"; \
		echo "      (git submodule update --init --depth 1, then re-run)"; \
	fi
	cd $(EXT_DIR) && $(NPM) run compile
	cd $(EXT_DIR) && $(NPX) --yes @vscode/vsce package --out sysml-v2.vsix
	@echo "packaged $(VSIX)"

vscode: vscode-package
	@if command -v $(CODE) >/dev/null 2>&1; then \
		$(CODE) --install-extension $(VSIX) --force; \
		echo "installed; reload VSCode windows to pick it up"; \
	else \
		echo "error: \`$(CODE)\` not on PATH -- install manually with:"; \
		echo "  code --install-extension $(VSIX)"; \
		exit 1; \
	fi

vscode-clean:
	rm -rf $(EXT_DIR)/node_modules $(EXT_DIR)/out $(EXT_DIR)/server \
		$(EXT_DIR)/library $(EXT_DIR)/*.vsix $(EXT_DIR)/package-lock.json
