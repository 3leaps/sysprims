#!/usr/bin/env bash
# Synthetic release-tooling controls: throwaway keys and local fixtures only, no
# operator keyring and no network. Needs gpg, git, node, python3 >= 3.11 and
# decernor >= 0.1.8 (SYSPRIMS_DECERNOR_BIN, or decernor on PATH for tests).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
for suite in release-tag-controls release-anchors release-validate-pin verify-pinned-tag release-tag \
	release-stage-public release-materialize-tag-messages verify-signatures release-crates; do
	echo "== $suite"
	"$root/scripts/$suite.test.sh"
done
echo '[ok] release tooling controls passed'
