# Conveniences for wiring the toolchain into an editor. `make vscode`
# builds the language server as a WebAssembly module, bundles it into
# the VSCode extension, packages the extension as a .vsix and installs
# it -- zero configuration afterwards. One package covers every machine
# and the browser, and the standard library is inside the module, so
# this works in a checkout that never fetched the submodule.
#
# The install is the one step that cannot always be done from here: it
# lands wherever this `code` points, and a checkout opened through a
# VSCode remote points at the server. `make vscode-package` is then the
# whole of what this machine can do, and the .vsix goes to the other
# side by hand.

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
# What VSCode knows the extension by, and whether it has a Node entry
# point -- read from the manifest so that neither can drift from it.
# Without a `main` it is a web extension: it runs in the worker
# extension host, which is on the side the editor itself runs on, and a
# remote server has nowhere to put it.
EXT_ID = $(shell node -p "const m = require('./$(EXT_DIR)/package.json'); m.publisher + '.' + m.name")
EXT_ON_SERVER = $(shell node -p "!!require('./$(EXT_DIR)/package.json').main")

.PHONY: help lsp wasm vscode vscode-package vscode-by-hand vscode-clean web \
	web-serve web-clean

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

# `code --install-extension` exits 0 whether or not it installed
# anything: it prints what it refused and returns a success. So what
# says whether this worked is the list of extensions afterwards, and
# never the status.
vscode: vscode-package
	@command -v $(CODE) >/dev/null 2>&1 || { \
		echo "error: \`$(CODE)\` not on PATH -- install manually with:"; \
		echo "  code --install-extension $(VSIX)"; \
		exit 1; \
	}
	@if [ -n "$$VSCODE_AGENT_FOLDER" ] && [ "$(EXT_ON_SERVER)" != "true" ]; then \
		echo "\`$(CODE)\` is the remote server's, and installs there. $(EXT_ID)"; \
		echo "has no \`main\`, so it runs in the worker extension host on the"; \
		echo "side the editor runs on and the server will not take it."; \
		$(MAKE) --no-print-directory vscode-by-hand; \
		exit 1; \
	fi
	-@$(CODE) --install-extension $(VSIX) --force
	@if $(CODE) --list-extensions 2>/dev/null | grep -Fqx "$(EXT_ID)"; then \
		echo "installed; reload VSCode windows to pick it up"; \
	else \
		echo "$(EXT_ID) is not installed: \`$(CODE)\` reported no error, and"; \
		echo "does not list it either."; \
		$(MAKE) --no-print-directory vscode-by-hand; \
		exit 1; \
	fi

# What is left to do when the install could not be done from here. The
# package itself is built and good: only the machine it goes on differs.
vscode-by-hand:
	@echo ""
	@echo "The package is built:"
	@echo "  $(VSIX)"
	@echo "Copy it to the machine VSCode itself runs on and install it there:"
	@echo "  code --install-extension sysml-v2.vsix"
	@echo "From this side, \`make vscode-package\` is the whole of what there"
	@echo "is to do."

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
