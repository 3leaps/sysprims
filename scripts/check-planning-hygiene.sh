#!/usr/bin/env bash
#
# Planning-reference hygiene guard.
#
# Planning artifacts (briefs, boards, task IDs, memos) live in a private,
# maintainer-managed system OUTSIDE this repository tree — see AGENTS.md →
# "Planning Artifacts". A .gitignore entry is a convenience filter, not a
# security boundary, so this guard also asserts that no *tracked* file
# reintroduces a reference to the private planning plane.
#
# Exclusions (each legitimately contains a guarded token):
#   - .gitignore names /.plans/ as defense-in-depth.
#   - this script names the patterns it guards.
set -euo pipefail

pattern='\.plans/|planning/|brief-sysp|SYSP-TASK|SYSP-[0-9]'

if git grep -nE "$pattern" \
	-- ':!.gitignore' ':!scripts/check-planning-hygiene.sh'; then
	echo "::error::A tracked file references the private planning plane. Keep" \
		"planning artifacts out of the repository tree (see AGENTS.md →" \
		"Planning Artifacts)."
	exit 1
fi

echo "OK: no private planning-plane references in tracked files."
