package recurrence

import (
	"sort"
	"strconv"
	"strings"
	"unicode/utf16"
)

// jcsObject maps member names to already-serialized RFC 8785 values.
type jcsObject map[string]string

func (o jcsObject) str(k, v string)     { o[k] = jcsString(v) }
func (o jcsObject) num(k string, n int) { o[k] = strconv.Itoa(n) }

func (o jcsObject) ints(k string, v []int) {
	if len(v) == 0 {
		return
	}
	s := make([]string, len(v))
	for i, n := range v {
		s[i] = strconv.Itoa(n)
	}
	o[k] = "[" + strings.Join(s, ",") + "]"
}

func (o jcsObject) locals(k string, v []Local) {
	if len(v) == 0 {
		return
	}
	s := make([]string, len(v))
	for i, l := range v {
		s[i] = jcsString(l.String())
	}
	o[k] = "[" + strings.Join(s, ",") + "]"
}

// render orders members by their UTF-16 code units, as RFC 8785 requires.
func (o jcsObject) render() string {
	keys := make([]string, 0, len(o))
	for k := range o {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool { return utf16Less(keys[i], keys[j]) })
	var b strings.Builder
	b.WriteByte('{')
	for i, k := range keys {
		if i > 0 {
			b.WriteByte(',')
		}
		b.WriteString(jcsString(k))
		b.WriteByte(':')
		b.WriteString(o[k])
	}
	b.WriteByte('}')
	return b.String()
}

func utf16Less(a, b string) bool {
	ua, ub := utf16.Encode([]rune(a)), utf16.Encode([]rune(b))
	for i := 0; i < len(ua) && i < len(ub); i++ {
		if ua[i] != ub[i] {
			return ua[i] < ub[i]
		}
	}
	return len(ua) < len(ub)
}

// jcsString escapes as ECMAScript JSON.stringify does: the two-character
// escapes for \b \t \n \f \r " \, \u00XX for other controls, and every other
// code point as itself.
func jcsString(s string) string {
	var b strings.Builder
	b.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			b.WriteString(`\"`)
		case '\\':
			b.WriteString(`\\`)
		case '\b':
			b.WriteString(`\b`)
		case '\t':
			b.WriteString(`\t`)
		case '\n':
			b.WriteString(`\n`)
		case '\f':
			b.WriteString(`\f`)
		case '\r':
			b.WriteString(`\r`)
		default:
			if r < 0x20 {
				const hexd = "0123456789abcdef"
				b.WriteString(`\u00`)
				b.WriteByte(hexd[r>>4])
				b.WriteByte(hexd[r&0xf])
			} else {
				b.WriteRune(r)
			}
		}
	}
	b.WriteByte('"')
	return b.String()
}
