package join

import "strings"

// RequestedName turns a hostname into the node name a join asks the hub for.
func RequestedName(hostname string) string {
	hostname = strings.ToLower(strings.SplitN(hostname, ".", 2)[0])
	var b strings.Builder
	for _, c := range hostname {
		if c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '-' {
			b.WriteRune(c)
		}
	}
	name := b.String()
	if name == "" || name[0] < 'a' || name[0] > 'z' {
		name = "node-" + name
	}
	if len(name) > 31 {
		name = name[:31]
	}
	switch name {
	case "mesh", "hub", "engine", "iii", "configuration", "harness", "shell", "coder":
		return "node-" + name
	}
	return name
}
