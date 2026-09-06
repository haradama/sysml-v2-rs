#!/usr/bin/env bash
# Checks a SysML/KerML file the moment Claude Code writes it, and hands
# the finding straight back to the model.
#
# The point is grounding: an LLM writing SysML v2 guesses at names, and
# this repository can say whether the guess is true. Silent when the file
# parses and every reference resolves.
#
# Reads the hook payload on stdin; writes PostToolUse `additionalContext`
# on stdout. Wired up in `.claude/settings.json`.
set -uo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

file="$(jq -r '.tool_input.file_path // .tool_response.filePath // empty')"
case "$file" in
*.sysml | *.kerml) ;;
*) exit 0 ;;
esac
[ -f "$file" ] || exit 0

# the CLI, however this checkout happens to have it built -- the newer
# of the two, since a stale binary predates the flags asked of it
release="$repo/target/release/sysml"
debug="$repo/target/debug/sysml"
if [ -x "$release" ] && { [ ! -x "$debug" ] || [ "$release" -nt "$debug" ]; }; then
	sysml=("$release")
elif [ -x "$debug" ]; then
	sysml=("$debug")
else
	sysml=(cargo run -q --manifest-path "$repo/Cargo.toml" -p sysml-cli --)
fi

# what the model is told, and nothing else
say() {
	jq -n --arg text "$1" '{
		hookSpecificOutput: {
			hookEventName: "PostToolUse",
			additionalContext: $text,
		},
	}'
	exit 0
}

# A run that answered in JSON made a finding. One that did not -- an
# older binary that has never heard of `--format`, a build that failed --
# is the checker's problem, not the file's, and inventing a fault for the
# file would be worse than saying nothing.
finding() {
	printf '%s' "$1" | jq -e . >/dev/null 2>&1
}

# syntax first: an unparsable file resolves to nonsense, and reporting
# that nonsense would only mislead
if ! parsed="$("${sysml[@]}" --format json parse "$file" 2>/dev/null)"; then
	finding "$parsed" || exit 0
	errors="$(printf '%s' "$parsed" | jq -c '[.files[].errors[]?]')"
	say "sysml parse: this file does not parse. $errors"
fi

# without the standard library every reference into it reads as
# unresolved, which would be a false alarm on every file
library="$repo/vendor/sysml-v2-release/sysml.library"
# A model of any size is spread over files, and a name this file uses
# is as likely declared in the one beside it. Checking the file alone
# reported every such name as resolving to nothing, so the directory
# is checked, and only this file's findings are reported.
dir="$(dirname "$file")"
args=(--format json check "$dir")
[ -d "$library" ] && args+=("$library")

if ! checked="$("${sysml[@]}" "${args[@]}" 2>/dev/null)"; then
	finding "$checked" || exit 0
	# only what this file is answerable for -- by the path the walk of
	# the directory spells it, which is the directory joined to the name
	mine="$(printf '%s' "$checked" |
		jq -c --arg f "$dir/$(basename "$file")" '[.unresolved[]? | select(.path == $f)]')"
	[ "$mine" = "[]" ] && exit 0
	say "sysml check: these references resolve to nothing. $mine"
fi
exit 0
