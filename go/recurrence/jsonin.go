package recurrence

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"math"
	"strconv"
	"unicode/utf8"
)

// jsonValue is a decoded JSON value: nil, bool, json.Number, string,
// []jsonValue or map[string]jsonValue.
type jsonValue = interface{}

var errDuplicateKey = errors.New("duplicate member name")

// decodeStrict decodes exactly one JSON value. It refuses duplicate member
// names at any depth (before an object could discard one), invalid UTF-8 and
// trailing data.
func decodeStrict(data []byte) (jsonValue, error) {
	if !utf8.Valid(data) {
		return nil, errors.New("invalid UTF-8")
	}
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.UseNumber()
	v, err := readValue(dec)
	if err != nil {
		return nil, err
	}
	if _, err := dec.Token(); err != io.EOF {
		return nil, errors.New("trailing data")
	}
	return v, nil
}

func readValue(dec *json.Decoder) (jsonValue, error) {
	tok, err := dec.Token()
	if err != nil {
		return nil, err
	}
	switch t := tok.(type) {
	case json.Delim:
		switch t {
		case '{':
			obj := map[string]jsonValue{}
			for dec.More() {
				kt, err := dec.Token()
				if err != nil {
					return nil, err
				}
				key, ok := kt.(string)
				if !ok {
					return nil, errors.New("object key")
				}
				if _, dup := obj[key]; dup {
					return nil, errDuplicateKey
				}
				val, err := readValue(dec)
				if err != nil {
					return nil, err
				}
				obj[key] = val
			}
			if _, err := dec.Token(); err != nil {
				return nil, err
			}
			return obj, nil
		case '[':
			arr := []jsonValue{}
			for dec.More() {
				val, err := readValue(dec)
				if err != nil {
					return nil, err
				}
				arr = append(arr, val)
			}
			if _, err := dec.Token(); err != nil {
				return nil, err
			}
			return arr, nil
		}
		return nil, errors.New("unexpected delimiter")
	default:
		return tok, nil
	}
}

// schemaInteger reports the value of a JSON Schema "integer": any number
// whose mathematical value is a whole number (so 2.0 qualifies).
func schemaInteger(v jsonValue) (int64, bool) {
	n, ok := v.(json.Number)
	if !ok {
		return 0, false
	}
	if i, err := strconv.ParseInt(string(n), 10, 64); err == nil {
		return i, true
	}
	f, err := strconv.ParseFloat(string(n), 64)
	if err != nil || math.IsInf(f, 0) || math.IsNaN(f) || f != math.Trunc(f) {
		return 0, false
	}
	// Whole numbers beyond int64 are integers, but outside every bound the
	// contract uses; clamp so range checks refuse them.
	if f > math.MaxInt32 {
		return math.MaxInt32, true
	}
	if f < math.MinInt32 {
		return math.MinInt32, true
	}
	return int64(f), true
}
