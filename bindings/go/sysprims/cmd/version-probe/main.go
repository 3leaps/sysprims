package main

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/3leaps/sysprims/bindings/go/sysprims"
)

type evidence struct {
	Schema        string `json:"schema"`
	Platform      string `json:"platform"`
	Version       string `json:"version"`
	FFIABIVersion uint32 `json:"ffi_abi_version"`
}

func main() {
	platform := os.Getenv("SYSPRIMS_PROBE_PLATFORM")
	expected := os.Getenv("SYSPRIMS_EXPECT_VERSION")
	if platform == "" || expected == "" {
		fmt.Fprintln(os.Stderr, "SYSPRIMS_PROBE_PLATFORM and SYSPRIMS_EXPECT_VERSION are required")
		os.Exit(2)
	}

	observed := sysprims.Version()
	if observed != expected {
		fmt.Fprintf(os.Stderr, "native version %q does not match expected %q\n", observed, expected)
		os.Exit(1)
	}
	abi := sysprims.ABIVersion()
	if abi == 0 {
		fmt.Fprintln(os.Stderr, "native FFI ABI version is zero")
		os.Exit(1)
	}

	if err := json.NewEncoder(os.Stdout).Encode(evidence{
		Schema:        "sysprims-go-smoke-evidence/v1",
		Platform:      platform,
		Version:       observed,
		FFIABIVersion: abi,
	}); err != nil {
		fmt.Fprintf(os.Stderr, "encode evidence: %v\n", err)
		os.Exit(1)
	}
}
