package hub

import "testing"

func TestInstallLines(t *testing.T) {
	unix, windows := InstallLines("0.1.0", "https://h", "ABCD-EFGH")
	if unix != "curl -fsSL https://raw.githubusercontent.com/iii-hq/workers/mesh/v0.1.0/mesh/install/install.sh | sh -s -- 0.1.0 https://h ABCD-EFGH" {
		t.Fatal(unix)
	}
	if windows != "& ([scriptblock]::Create((irm https://raw.githubusercontent.com/iii-hq/workers/mesh/v0.1.0/mesh/install/install.ps1))) 0.1.0 https://h ABCD-EFGH" {
		t.Fatal(windows)
	}
}
