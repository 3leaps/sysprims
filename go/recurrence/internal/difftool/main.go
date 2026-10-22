// Command difftool is the Go side of the cross-language differential
// driver: one JSON rule per stdin line, one result line per rule.
package main

import (
	"bufio"
	"fmt"
	"os"
	"strings"

	"github.com/3leaps/sysprims/go/recurrence"
)

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<20)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	windows := []struct {
		from, to string
		limit    int
	}{
		{"2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z", 40},
		{"2026-03-01T00:00:00Z", "2026-04-15T00:00:00Z", 25},
		{"2026-10-01T00:00:00Z", "2026-11-15T00:00:00Z", 25},
	}
	for in.Scan() {
		r, err := recurrence.ParseJSON(in.Bytes())
		if err != nil {
			fmt.Fprintf(out, "refused %s\n", recurrence.ReasonOf(err))
			continue
		}
		parts := []string{r.Digest(), strings.Join(r.Text(), "|")}
		for _, w := range windows {
			from, _ := recurrence.ParseInstant(w.from)
			to, _ := recurrence.ParseInstant(w.to)
			got, err := r.Expand(from, to, w.limit)
			if err != nil {
				parts = append(parts, "error:"+err.Error())
				continue
			}
			s := make([]string, len(got))
			for i, t := range got {
				s[i] = recurrence.FormatInstant(t)
			}
			parts = append(parts, strings.Join(s, ","))
		}
		after, _ := recurrence.ParseInstant("2026-06-15T12:00:00Z")
		next, found, err := r.NextAfter(after)
		switch {
		case err != nil:
			parts = append(parts, "error:"+err.Error())
		case found:
			parts = append(parts, recurrence.FormatInstant(next))
		default:
			parts = append(parts, "none")
		}
		fmt.Fprintln(out, strings.Join(parts, " "))
	}
}
