package drives

import "testing"

func TestMountTableResolvesBindOnSeparateFilesystem(t *testing.T) {
	at := parseMounts(`1 0 8:1 / / rw - ext4 /dev/root rw
2 1 8:2 / /srv rw - ext4 /dev/data rw
3 1 8:2 /iii-mesh/proj /drives/proj rw - ext4 /dev/data rw
4 1 0:9 / /drives/remote rw - nfs4 [fd10:20:30::1]:/srv/iii-mesh/remote rw
`)
	if at["/drives/proj"] != "/srv/iii-mesh/proj" || at["/drives/remote"] != "[fd10:20:30::1]:/srv/iii-mesh/remote" {
		t.Fatal(at)
	}
}
