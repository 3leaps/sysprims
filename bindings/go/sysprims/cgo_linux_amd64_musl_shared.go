//go:build linux && amd64 && musl && sysprims_shared

package sysprims

/*
#error "sysprims_shared is not available on musl yet (see Feature 04: musl shared artifacts)"
*/
import "C"
