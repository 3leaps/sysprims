// SPDX-License-Identifier: MIT OR Apache-2.0

// Command observecli emits canonical JSON native-process-observation:v0 payloads
// for cross-language live parity checks against crates/sysprims-proc.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strconv"

	"github.com/3leaps/sysprims/evidence"
	"github.com/3leaps/sysprims/procobserve"
)

const defaultDerivationRev = "sha256:0000000000000000000000000000000000000000000000000000000000000000"

func parsePIDFlag(s string) (uint32, bool) {
	v, err := strconv.ParseUint(s, 10, 32)
	if err != nil || v == 0 || v > uint64(^uint32(0)) {
		return 0, false
	}
	return uint32(v), true
}

func main() {
	var (
		pidFlag      = flag.String("pid", "", "target numeric PID (1..=2147483647)")
		observedAt   = flag.String("observed-at", "", "explicit UTC timestamp")
		subjectRef   = flag.String("subject-ref", "process-target", "report-local process subject ID")
		allContext   = flag.Bool("all-context", false, "opt in to all execution-context facts")
		noHostIDRef  = flag.Bool("no-host-id-ref", false, "omit host_scope.identifier_ref")
		hostIDRef    = flag.String("host-id-ref", "id-host-local", "host_scope.identifier_ref")
		hostNS       = flag.String("host-namespace", "urn:3leaps:identity-evidence:host-scope:test-fixture:v0", "host_scope.namespace")
		hostVal      = flag.String("host-value", "local-qualification-host", "host_scope.value")
		hostDerivID  = flag.String("host-derivation-id", "urn:3leaps:identity-evidence:derivation:caller-supplied-host-scope:v0", "host_scope.derivation.id")
		hostDerivRev = flag.String("host-derivation-rev", defaultDerivationRev, "host_scope.derivation.revision")
		hostStab     = flag.String("host-stability", "boot", "host_scope.stability")
		hostClone    = flag.String("host-clone-behavior", "regenerated", "host_scope.clone_behavior")
	)
	flag.Parse()

	targetPID, ok := parsePIDFlag(*pidFlag)
	if !ok {
		fmt.Fprintln(os.Stderr, "error: --pid <1..=2147483647> is required")
		os.Exit(2)
	}

	idRef := *hostIDRef
	if *noHostIDRef {
		idRef = ""
	}
	hs := procobserve.HostScope{
		IdentifierRef: idRef,
		Namespace:     *hostNS,
		Value:         *hostVal,
		Derivation: evidence.ProfileRef{
			ID:       *hostDerivID,
			Revision: *hostDerivRev,
		},
		Stability:     *hostStab,
		CloneBehavior: *hostClone,
	}

	opts := procobserve.DefaultObservationOptions()
	opts.SubjectRef = *subjectRef
	opts.ObservedAt = *observedAt
	if *allContext {
		opts = opts.WithAllContextFacts()
	}

	obs, err := procobserve.ObserveProcess(targetPID, hs, opts)
	if err != nil {
		fmt.Fprintf(os.Stderr, "refused:%v\n", err)
		os.Exit(1)
	}

	raw, err := json.Marshal(obs)
	if err != nil {
		fmt.Fprintf(os.Stderr, "serialization-error:%v\n", err)
		os.Exit(1)
	}
	admitted, err := evidence.AdmitReportBytes(raw)
	if err != nil {
		fmt.Fprintf(os.Stderr, "canonicalization-error:%v\n", err)
		os.Exit(1)
	}
	_, _ = os.Stdout.Write(admitted.CanonicalBytes)
	_, _ = os.Stdout.Write([]byte("\n"))
}
