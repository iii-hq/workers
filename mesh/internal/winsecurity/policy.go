// Package winsecurity enforces the separate Windows private-state and public-artifact boundaries.
package winsecurity

import "fmt"

const (
	systemSID                  = "S-1-5-18"
	adminsSID                  = "S-1-5-32-544"
	usersSID                   = "S-1-5-32-545"
	trustedInstallerSID        = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
	fullAccess          uint32 = 0x1f01ff
	readExecute         uint32 = 0x1200a9
	PrivateSDDL                = "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
	PublicSDDL                 = "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)"
)

type ace struct {
	sid         string
	mask        uint32
	inheritOnly bool
}

// File objects map generic rights even when an inheritable ACE retains its
// generic form. Validate both forms identically, including inherit-only grants.
func fileRights(mask uint32) uint32 {
	for _, mapping := range []struct{ generic, file uint32 }{
		{0x80000000, 0x120089}, // GENERIC_READ -> FILE_GENERIC_READ
		{0x40000000, 0x120116}, // GENERIC_WRITE -> FILE_GENERIC_WRITE
		{0x20000000, 0x1200a0}, // GENERIC_EXECUTE -> FILE_GENERIC_EXECUTE
		{0x10000000, fullAccess},
	} {
		if mask&mapping.generic != 0 {
			mask = mask & ^mapping.generic | mapping.file
		}
	}
	return mask
}

func validateACL(owner string, protected bool, acl []ace, public, ancestor bool) error {
	if owner != adminsSID && owner != systemSID && !(ancestor && owner == trustedInstallerSID) {
		return fmt.Errorf("untrusted Windows path owner %s", owner)
	}
	if len(acl) == 0 || (!ancestor && !protected) {
		return fmt.Errorf("Windows path requires a protected nonempty DACL")
	}
	var system, admins, readers bool
	for _, a := range acl {
		if ancestor && a.inheritOnly {
			continue
		}
		// Keep the stricter existing generic-write ancestor rule. Boundary
		// objects need the file mapping for effective and inheritable grants.
		if !ancestor {
			a.mask = fileRights(a.mask)
		}
		if a.sid == systemSID {
			system = system || !a.inheritOnly && a.mask&fullAccess == fullAccess
			continue
		}
		if a.sid == adminsSID {
			admins = admins || !a.inheritOnly && a.mask&fullAccess == fullAccess
			continue
		}
		if ancestor && a.sid == trustedInstallerSID {
			continue
		}
		if ancestor {
			// Creating a sibling cannot replace a protected child. Deleting it or
			// rewriting the parent ACL can, so those rights are never accepted.
			if a.mask&(0x10000000|0x40000000|0xd0040) != 0 {
				return fmt.Errorf("untrusted ancestor deletion or ACL access")
			}
			continue
		}
		if !public || a.sid != usersSID || a.mask & ^readExecute != 0 {
			return fmt.Errorf("unexpected access for %s", a.sid)
		}
		readers = readers || !a.inheritOnly && a.mask&readExecute == readExecute
	}
	if !ancestor && (!system || !admins || public && !readers) {
		return fmt.Errorf("missing required Windows path access")
	}
	return nil
}
