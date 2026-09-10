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
	$(CARGO) build --release -p sysml-lsp

$(EXT_DIR)/node_modules: $(EXT_DIR)/package.json
	cd $(EXT_DIR) && $(NPM) install --no-audit --no-fund

# the extension looks for server/sysml-lsp and library/sysml.library next
# to its own files before falling back to settings and PATH
vscode-package: lsp $(EXT_DIR)/node_modules
	mkdir -p $(EXT_DIR)/server
	cp $(SERVER) $(EXT_DIR)/server/
	rm -rf $(EXT_DIR)/library
	mkdir -p $(EXT_DIR)/library/sysml.library
	cp -R $(LIBRARY)/. $(EXT_DIR)/library/sysml.library/
	@# The package declares two licences and used to carry neither, which
	@# is what stops `vsce package` to ask whether to go on without one.
	@# All three texts travel in it instead: the extension's own two, and
	@# the one the standard library is under. Bundling `library/` makes
	@# this package a distributor of EPL-2.0 content, and EPL-2.0 asks a
	@# distributor to pass the licence and the notice along -- so leaving
	@# it out was not a tidiness question.
	@printf '%s\n' \
		'The SysML v2 extension is part of sysml-v2-rs, which is licensed' \
		'under either of the Apache License 2.0 or the MIT license, at your' \
		'option.' \
		'' \
		'It also carries, under `library/`, the KerML and SysML v2 standard' \
		'model libraries, taken unchanged from the OMG SysML v2 Release' \
		'(https://github.com/Systems-Modeling/SysML-v2-Release) and licensed' \
		'under the Eclipse Public License 2.0. Nothing in `library/` was' \
		'written here and nothing in it was changed.' \
		'' \
		'The text of all three follows.' \
		'' '=== MIT ===' '' > $(EXT_DIR)/LICENSE.txt
	@cat LICENSE-MIT >> $(EXT_DIR)/LICENSE.txt
	@printf '\n%s\n\n' '=== Apache License 2.0 ===' >> $(EXT_DIR)/LICENSE.txt
	@cat LICENSE-APACHE >> $(EXT_DIR)/LICENSE.txt
	@printf '\n%s\n\n' '=== Eclipse Public License 2.0 (for library/) ===' \
		>> $(EXT_DIR)/LICENSE.txt
	@cat $(LIBRARY)/../LICENSE >> $(EXT_DIR)/LICENSE.txt
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
