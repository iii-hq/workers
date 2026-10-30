package winsecurity

import "testing"

func TestWindowsACLPolicy(t *testing.T) {
	private := []ace{{sid: systemSID, mask: fullAccess}, {sid: adminsSID, mask: fullAccess}}
	public := append(append([]ace{}, private...), ace{sid: usersSID, mask: readExecute})
	for _, tc := range []struct {
		name, owner                 string
		protected, public, ancestor bool
		acl                         []ace
		valid                       bool
	}{
		{"private", adminsSID, true, false, false, private, true},
		{"public", adminsSID, true, true, false, public, true},
		{"secret readable", adminsSID, true, false, false, public, false},
		{"public writable", adminsSID, true, true, false, append(append([]ace{}, public...), ace{sid: usersSID, mask: 2}), false},
		{"unprotected", adminsSID, false, false, false, private, false},
		{"operator owner", usersSID, true, true, false, public, false},
		{"null dacl", adminsSID, true, false, false, nil, false},
		{"no operator access", adminsSID, true, true, false, private, false},
		{"parent create", adminsSID, false, false, true, append(append([]ace{}, public...), ace{sid: usersSID, mask: 4}), true},
		{"parent delete child", adminsSID, false, false, true, append(append([]ace{}, public...), ace{sid: usersSID, mask: 64}), false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := validateACL(tc.owner, tc.protected, tc.acl, tc.public, tc.ancestor)
			if (err == nil) != tc.valid {
				t.Fatalf("valid=%v error=%v", tc.valid, err)
			}
		})
	}
}

func TestWindowsACLInheritancePolicy(t *testing.T) {
	base := []ace{{sid: systemSID, mask: fullAccess}, {sid: adminsSID, mask: fullAccess}}
	inherited := append(append([]ace{}, base...), ace{sid: usersSID, mask: fullAccess, inheritOnly: true})
	if err := validateACL(adminsSID, true, inherited, false, false); err == nil {
		t.Fatal("accepted an ACL leaking new private children")
	}
	if err := validateACL(adminsSID, false, inherited, false, true); err != nil {
		t.Fatal("protected child may safely bypass ancestor inheritance", err)
	}
	onlyInheritedRead := append(base, ace{sid: usersSID, mask: readExecute, inheritOnly: true})
	if err := validateACL(adminsSID, true, onlyInheritedRead, true, false); err == nil {
		t.Fatal("cache grants no effective traversal")
	}
}

func TestWindowsACLGenericFileRights(t *testing.T) {
	base := []ace{{sid: systemSID, mask: fullAccess}, {sid: adminsSID, mask: fullAccess}}
	for _, tc := range []struct {
		name   string
		public bool
		users  []ace
		valid  bool
	}{
		{"generic effective", true, []ace{{usersSID, 0xa0000000, false}}, true},
		// Windows retains the generic inherit-only ACE alongside mapped access.
		{"mapped effective and generic inheritance", true, []ace{{usersSID, readExecute, false}, {usersSID, 0xa0000000, true}}, true},
		{"generic inheritance lacks traversal", true, []ace{{usersSID, 0xa0000000, true}}, false},
		{"generic write", true, []ace{{usersSID, 0xe0000000, false}}, false},
		{"generic inherited write", true, []ace{{usersSID, readExecute, false}, {usersSID, 0xe0000000, true}}, false},
		{"generic all", true, []ace{{usersSID, 0x10000000, false}}, false},
		{"private generic read", false, []ace{{usersSID, 0x80000000, false}}, false},
		{"private inherited generic read", false, []ace{{usersSID, 0x80000000, true}}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			acl := append(append([]ace{}, base...), tc.users...)
			if err := validateACL(adminsSID, true, acl, tc.public, false); (err == nil) != tc.valid {
				t.Fatalf("valid=%v error=%v", tc.valid, err)
			}
		})
	}
}
