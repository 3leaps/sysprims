#!/usr/bin/env bash
# Synthetic release-tooling controls: throwaway keys and local fixtures only, no
# operator keyring and no network. Needs gpg, minisign, git, node, python3 >= 3.11 with PyYAML and
# decernor >= 0.1.8 (SYSPRIMS_DECERNOR_BIN, or decernor on PATH for tests).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
for suite in release-tag-controls release-anchors release-validate-pin verify-pinned-tag release-tag \
	release-stage-public release-fetch-anchors release-materialize-tag-messages verify-signatures release-crates \
	release-fetch-ci-artifacts workflow-permissions workflow-pins; do
	echo "== $suite"
	"$root/scripts/$suite.test.sh"
done
echo '[ok] release tooling controls passed'
