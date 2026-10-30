package hub

import (
	"fmt"
	"regexp"
	"strings"
)

var bareArgument = regexp.MustCompile(`^[a-zA-Z0-9_./:@-]+$`)

func shellArgument(s string) string {
	if bareArgument.MatchString(s) {
		return s
	}
	return "'" + strings.ReplaceAll(s, "'", "'\"'\"'") + "'"
}
func powershellArgument(s string) string {
	if bareArgument.MatchString(s) {
		return s
	}
	return "'" + strings.ReplaceAll(s, "'", "''") + "'"
}
func InstallLines(version, publicURL, code string) (unix, windows string) {
	base := "https://raw.githubusercontent.com/iii-hq/workers/mesh/v" + version + "/mesh/install/install."
	return fmt.Sprintf("curl -fsSL %s | sh -s -- %s %s %s", shellArgument(base+"sh"), shellArgument(version), shellArgument(publicURL), shellArgument(code)),
		fmt.Sprintf("& ([scriptblock]::Create((irm %s))) %s %s %s", powershellArgument(base+"ps1"), powershellArgument(version), powershellArgument(publicURL), powershellArgument(code))
}
