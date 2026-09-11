# Conveniences for wiring the toolchain into an editor. `make vscode`
# builds the language server, bundles it and the standard library into
# the VSCode extension, packages the extension as a .vsix and installs
# it -- zero configuration afterwards. The library comes from
# `sysml-stdlib` rather than from the submodule, so this works in a
# checkout that never fetched one.

CARGO ?= cargo
NPM ?= npm
NPX ?= npx
CODE ?= code

EXT_DIR := editors/vscode
# The copy that ships, not the submodule's: `sysml-stdlib` carries the
# standard library and the language server has it built in, so this is
# what the extension would be pointed at anyway.
LIBRARY := crates/sysml-stdlib/library
SERVER := target/release/sysml-lsp
VSIX := $(EXT_DIR)/sysml-v2.vsix

.PHONY: help lsp vscode vscode-package vscode-clean

help:
	@echo "make vscode          build, package and install the VSCode extension"
	@echo "make vscode-package  build the .vsix without installing it"
	@echo "make lsp             build the language server only"
	@echo "make vscode-clean    remove the extension's build artifacts"

lsp:
	$(CARGO) build --release -p sysmlv2-lsp

$(EXT_DIR)/node_modules: $(EXT_DIR)/package.json
	cd $(EXT_DIR) && $(NPM) install --no-audit --no-fund

# `bundle.mjs` puts the server and the standard library where the
# extension looks for them, and writes the licences that travel with them
vscode-package: lsp $(EXT_DIR)/node_modules
	node $(EXT_DIR)/scripts/bundle.mjs $(SERVER)
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
		$(EXT_DIR)/library $(EXT_DIR)/*.vsix $(EXT_DIR)/package-lock.json \
		$(EXT_DIR)/LICENSE.txt
