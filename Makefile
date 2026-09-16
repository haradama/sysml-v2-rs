# Conveniences for wiring the toolchain into an editor. `make vscode`
# builds the language server as a WebAssembly module, bundles it into
# the VSCode extension, packages the extension as a .vsix and installs
# it -- zero configuration afterwards. One package covers every machine
# and the browser, and the standard library is inside the module, so
# this works in a checkout that never fetched the submodule.

CARGO ?= cargo
NPM ?= npm
NPX ?= npx
CODE ?= code

EXT_DIR := editors/vscode
WEB_DIR := web
# The WebAssembly module the extension ships, built with the profile the
# root Cargo.toml keeps for it.
WASM_TARGET := wasm32-unknown-unknown
SERVER := target/$(WASM_TARGET)/wasm/sysml_wasm.wasm
VSIX := $(EXT_DIR)/sysml-v2.vsix

.PHONY: help lsp wasm vscode vscode-package vscode-clean web web-serve web-clean

help:
	@echo "make vscode          build, package and install the VSCode extension"
	@echo "make vscode-package  build the .vsix without installing it"
	@echo "make web             build the playground into web/dist"
	@echo "make web-serve       build it and serve it at http://localhost:8000"
	@echo "make wasm            build the language server as a WebAssembly module"
	@echo "make lsp             build the language server as a program"
	@echo "make vscode-clean    remove the extension's build artifacts"
	@echo "make web-clean       remove the playground's build artifacts"

lsp:
	$(CARGO) build --release -p sysmlv2-lsp

# `rustup target add wasm32-unknown-unknown` is the whole toolchain
wasm:
	$(CARGO) build -p sysmlv2-wasm --target $(WASM_TARGET) --profile wasm

$(EXT_DIR)/node_modules: $(EXT_DIR)/package.json
	cd $(EXT_DIR) && $(NPM) install --no-audit --no-fund

# `bundle.mjs` puts the module where the extension looks for it and
# writes the licences that travel with it; `npm run compile` type-checks
# the TypeScript and bundles it
vscode-package: wasm $(EXT_DIR)/node_modules
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

$(WEB_DIR)/node_modules: $(WEB_DIR)/package.json
	cd $(WEB_DIR) && $(NPM) install --no-audit --no-fund

# The playground: the same module the extension ships, in a page of its
# own. `web/dist` is the whole site -- there is nothing to run beside it.
web: wasm $(WEB_DIR)/node_modules
	cd $(WEB_DIR) && $(NPM) run build

web-serve: wasm $(WEB_DIR)/node_modules
	cd $(WEB_DIR) && $(NPM) run serve

web-clean:
	rm -rf $(WEB_DIR)/node_modules $(WEB_DIR)/dist $(WEB_DIR)/package-lock.json
