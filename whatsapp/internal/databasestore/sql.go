package databasestore

import (
	"database/sql/driver"
	"encoding/hex"
	"errors"
	"strconv"
	"strings"
)

func owned(name string) bool {
	return strings.HasPrefix(name, "inbox_") || strings.HasPrefix(name, "whatsmeow_") || strings.HasPrefix(name, "idx_whatsmeow_") || name == "storage_migration"
}

type token struct {
	text string
	kind byte
}

// Only SQL shipped with the worker/library crosses this adapter. Lexing keeps
// identifiers, strings, comments and numbered placeholders separate.
func tokens(query string) ([]token, error) {
	var out []token
	for i := 0; i < len(query); {
		start := i
		ch := query[i]
		kind := byte(0)
		switch {
		case ch == '\'' || ch == '"' || ch == '`' || ch == '[':
			kind = ch
			end := ch
			if ch == '[' {
				end = ']'
			}
			i++
			closed := false
			for i < len(query) {
				if query[i] == end {
					i++
					if i < len(query) && query[i] == end {
						i++
						continue
					}
					closed = true
					break
				}
				i++
			}
			if !closed {
				return nil, errors.New("unterminated SQL quote")
			}
		case ch == '-' && i+1 < len(query) && query[i+1] == '-':
			kind = 'c'
			i += 2
			for i < len(query) && query[i] != '\n' {
				i++
			}
		case ch == '/' && i+1 < len(query) && query[i+1] == '*':
			kind = 'c'
			i += 2
			end := strings.Index(query[i:], "*/")
			if end < 0 {
				return nil, errors.New("unterminated SQL comment")
			}
			i += end + 2
		case letter(ch):
			kind = 'i'
			i++
			for i < len(query) && (letter(query[i]) || digit(query[i])) {
				i++
			}
		case ch == '?' || ch == '$':
			kind = 'p'
			i++
			for i < len(query) && digit(query[i]) {
				i++
			}
		default:
			i++
		}
		out = append(out, token{query[start:i], kind})
	}
	return out, nil
}
func letter(ch byte) bool { return ch == '_' || ch >= 'a' && ch <= 'z' || ch >= 'A' && ch <= 'Z' }
func digit(ch byte) bool  { return ch >= '0' && ch <= '9' }
func rewrite(query, prefix string, args []driver.NamedValue) (string, []any, error) {
	ts, err := tokens(query)
	if err != nil {
		return "", nil, err
	}
	params := make([]any, len(args))
	binary := make([]bool, len(args))
	for i, arg := range args {
		if arg.Name != "" {
			return "", nil, errors.New("named SQL parameters are not supported")
		}
		if b, ok := arg.Value.([]byte); ok {
			if b == nil {
				params[i] = nil
			} else {
				params[i] = hex.EncodeToString(b)
				binary[i] = true
			}
		} else {
			params[i] = arg.Value
		}
	}
	// dbutil introspects table names as bound values rather than identifiers.
	if strings.Contains(strings.ToLower(query), "pragma_table_info(") || strings.Contains(strings.ToLower(query), "tbl_name=") || strings.Contains(strings.ToLower(query), "tbl_name =") {
		if len(params) > 0 {
			if name, ok := params[0].(string); ok && owned(name) {
				params[0] = prefix + name
			}
		}
	}
	var b strings.Builder
	ordinal := 0
	for _, t := range ts {
		switch t.kind {
		case 'i':
			if owned(t.text) {
				b.WriteString(prefix)
			}
			b.WriteString(t.text)
		case '"', '`', '[':
			name := t.text[1 : len(t.text)-1]
			if owned(name) {
				b.WriteByte(t.text[0])
				b.WriteString(prefix)
				b.WriteString(t.text[1:])
			} else {
				b.WriteString(t.text)
			}
		case '\'':
			name := t.text[1 : len(t.text)-1]
			// Schema introspection uses SQL literals in Inbox and dbutil.
			if owned(name) && (strings.Contains(strings.ToLower(query), "pragma_table_info(") || strings.Contains(strings.ToLower(query), "sqlite_master")) {
				b.WriteByte('\'')
				b.WriteString(prefix)
				b.WriteString(t.text[1:])
			} else {
				b.WriteString(t.text)
			}
		case 'p':
			if len(t.text) > 1 {
				ordinal, err = strconv.Atoi(t.text[1:])
				if err != nil {
					return "", nil, errors.New("invalid SQL placeholder")
				}
			} else {
				ordinal++
			}
			if ordinal < 1 || ordinal > len(params) {
				return "", nil, errors.New("missing SQL parameter")
			}
			placeholder := "?" + strconv.Itoa(ordinal)
			if binary[ordinal-1] {
				b.WriteString("unhex(" + placeholder + ")")
			} else {
				b.WriteString(placeholder)
			}
		default:
			b.WriteString(t.text)
		}
	}
	return b.String(), params, nil
}
func splitSQL(query string) ([]string, error) {
	ts, err := tokens(query)
	if err != nil {
		return nil, err
	}
	var out []string
	var b strings.Builder
	hasSQL := false
	for _, t := range ts {
		if t.text == ";" && t.kind == 0 {
			if hasSQL {
				out = append(out, b.String())
			}
			b.Reset()
			hasSQL = false
			continue
		}
		b.WriteString(t.text)
		if t.kind != 'c' && strings.TrimSpace(t.text) != "" {
			hasSQL = true
		}
	}
	if hasSQL {
		out = append(out, b.String())
	}
	return out, nil
}
