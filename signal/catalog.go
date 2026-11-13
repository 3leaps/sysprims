// SPDX-License-Identifier: MIT OR Apache-2.0

package signal

import (
	"fmt"
	"strings"

	"github.com/3leaps/sysprims/syserror"
)

// Standard signal constants matching the Crucible / rsfulmen signal catalog.
const (
	SIGHUP  = 1
	SIGINT  = 2
	SIGQUIT = 3
	SIGKILL = 9
	SIGPIPE = 13
	SIGALRM = 14
	SIGTERM = 15
)

type catalogEntry struct {
	id     string
	name   string
	number int
}

var signalCatalog = []catalogEntry{
	{id: "term", name: "SIGTERM", number: SIGTERM},
	{id: "int", name: "SIGINT", number: SIGINT},
	{id: "hup", name: "SIGHUP", number: SIGHUP},
	{id: "quit", name: "SIGQUIT", number: SIGQUIT},
	{id: "pipe", name: "SIGPIPE", number: SIGPIPE},
	{id: "alrm", name: "SIGALRM", number: SIGALRM},
	{id: "usr1", name: "SIGUSR1", number: SIGUSR1},
	{id: "usr2", name: "SIGUSR2", number: SIGUSR2},
	{id: "kill", name: "SIGKILL", number: SIGKILL},
}

// ListSignalNames returns the canonical signal names in catalog order.
func ListSignalNames() []string {
	out := make([]string, len(signalCatalog))
	for i, s := range signalCatalog {
		out[i] = s.name
	}
	return out
}

func lookupByNameExact(name string) (int, bool) {
	for _, s := range signalCatalog {
		if s.name == name {
			return s.number, true
		}
	}
	return 0, false
}

func lookupByIDExact(id string) (int, bool) {
	for _, s := range signalCatalog {
		if s.id == id {
			return s.number, true
		}
	}
	return 0, false
}

// ResolveSignalName resolves a signal number by canonical name ("SIGTERM"),
// case-insensitive name ("sigterm"), unprefixed name ("TERM"), or short ID
// ("term").
func ResolveSignalName(signalName string) (int, error) {
	trimmed := strings.TrimSpace(signalName)
	if trimmed == "" {
		return 0, syserror.InvalidArgument(fmt.Sprintf("unknown signal name: %s", signalName))
	}

	if num, ok := lookupByNameExact(trimmed); ok {
		return num, nil
	}

	upper := strings.ToUpper(trimmed)
	if num, ok := lookupByNameExact(upper); ok {
		return num, nil
	}

	if !strings.HasPrefix(upper, "SIG") {
		if num, ok := lookupByNameExact("SIG" + upper); ok {
			return num, nil
		}
	}

	lower := strings.ToLower(trimmed)
	if num, ok := lookupByIDExact(lower); ok {
		return num, nil
	}

	return 0, syserror.InvalidArgument(fmt.Sprintf("unknown signal name: %s", signalName))
}

// MatchSignalNames returns canonical signal names matching a simple glob
// pattern ('*' for any sequence, '?' for single character), matching ASCII
// case-insensitively against both the signal name and short ID.
func MatchSignalNames(pattern string) []string {
	trimmed := strings.TrimSpace(pattern)
	if trimmed == "" {
		return []string{}
	}
	pat := strings.ToLower(trimmed)
	matches := make([]string, 0)
	for _, s := range signalCatalog {
		nameLower := strings.ToLower(s.name)
		idLower := strings.ToLower(s.id)
		if globMatch(pat, nameLower) || globMatch(pat, idLower) {
			already := false
			for _, m := range matches {
				if m == s.name {
					already = true
					break
				}
			}
			if !already {
				matches = append(matches, s.name)
			}
		}
	}
	return matches
}

func globMatch(pattern, text string) bool {
	pIdx := 0
	tIdx := 0
	starIdx := -1
	matchIdx := 0

	for tIdx < len(text) {
		if pIdx < len(pattern) && (pattern[pIdx] == '?' || pattern[pIdx] == text[tIdx]) {
			pIdx++
			tIdx++
			continue
		}
		if pIdx < len(pattern) && pattern[pIdx] == '*' {
			starIdx = pIdx
			matchIdx = tIdx
			pIdx++
			continue
		}
		if starIdx != -1 {
			pIdx = starIdx + 1
			matchIdx++
			tIdx = matchIdx
			continue
		}
		return false
	}

	for pIdx < len(pattern) && pattern[pIdx] == '*' {
		pIdx++
	}

	return pIdx == len(pattern)
}
