package winsecurity

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"unsafe"

	"golang.org/x/sys/windows"
)

func ProgramData() string {
	p, err := windows.KnownFolderPath(windows.FOLDERID_ProgramData, 0)
	if err != nil {
		return ""
	}
	return p
}
func ProgramFiles() string {
	p, err := windows.KnownFolderPath(windows.FOLDERID_ProgramFiles, 0)
	if err != nil {
		return ""
	}
	return p
}

func openPath(path string) (windows.Handle, error) {
	p, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return 0, err
	}
	h, err := windows.CreateFile(p, windows.READ_CONTROL|windows.FILE_READ_ATTRIBUTES, windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE, nil, windows.OPEN_EXISTING, windows.FILE_FLAG_BACKUP_SEMANTICS|windows.FILE_FLAG_OPEN_REPARSE_POINT, 0)
	if err != nil {
		return 0, err
	}
	var info windows.ByHandleFileInformation
	if err = windows.GetFileInformationByHandle(h, &info); err != nil || info.FileAttributes&windows.FILE_ATTRIBUTE_REPARSE_POINT != 0 {
		windows.CloseHandle(h)
		return 0, errors.Join(err, fmt.Errorf("unsafe reparse path %s", path))
	}
	return h, nil
}
func checkHandle(h windows.Handle, public, ancestor bool) error {
	sd, err := windows.GetSecurityInfo(h, windows.SE_FILE_OBJECT, windows.OWNER_SECURITY_INFORMATION|windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		return err
	}
	owner, _, err := sd.Owner()
	if err != nil || owner == nil {
		return errors.Join(err, errors.New("missing Windows path owner"))
	}
	control, _, err := sd.Control()
	if err != nil {
		return err
	}
	acl, _, err := sd.DACL()
	if err != nil || acl == nil {
		return errors.Join(err, errors.New("missing Windows DACL"))
	}
	var entries []ace
	for i := uint32(0); i < uint32(acl.AceCount); i++ {
		var a *windows.ACCESS_ALLOWED_ACE
		if err = windows.GetAce(acl, i, &a); err != nil {
			return err
		}
		if a.Header.AceType != windows.ACCESS_ALLOWED_ACE_TYPE {
			return errors.New("unsupported Windows access rule")
		}
		sid := (*windows.SID)(unsafe.Pointer(&a.SidStart))
		entries = append(entries, ace{sid.String(), uint32(a.Mask), a.Header.AceFlags&windows.INHERIT_ONLY_ACE != 0})
	}
	// Child files inherit the protected parent's DACL; inheritance is safe only
	// after Check has validated the entire path, including its protected parent.
	return validateACL(owner.String(), control&windows.SE_DACL_PROTECTED != 0 || !ancestor, entries, public, ancestor)
}
func ancestors(path string) ([]windows.Handle, error) {
	if !filepath.IsAbs(path) || filepath.VolumeName(path) == "" || len(filepath.VolumeName(path)) != 2 {
		return nil, errors.New("Windows service paths must be absolute local drive paths")
	}
	parent := filepath.Dir(path)
	var handles []windows.Handle
	if parent != path {
		var err error
		handles, err = ancestors(parent)
		if err != nil {
			return nil, err
		}
	}
	h, err := openPath(path)
	if err == nil {
		err = checkHandle(h, false, true)
	}
	if err != nil {
		if h != 0 {
			windows.CloseHandle(h)
		}
		closeHandles(handles)
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return append(handles, h), nil
}
func closeHandles(hs []windows.Handle) {
	for i := len(hs) - 1; i >= 0; i-- {
		windows.CloseHandle(hs[i])
	}
}

// EnsureDirectory creates the final directory with its protected DACL already
// applied, while holding ancestor handles without delete sharing.
func EnsureDirectory(path string, public bool) error {
	path = filepath.Clean(path)
	hs, err := ancestors(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer closeHandles(hs)
	h, err := openPath(path)
	if errors.Is(err, windows.ERROR_FILE_NOT_FOUND) || errors.Is(err, windows.ERROR_PATH_NOT_FOUND) {
		sddl := PrivateSDDL
		if public {
			sddl = PublicSDDL
		}
		sd, e := windows.SecurityDescriptorFromString(sddl)
		if e != nil {
			return e
		}
		p, e := windows.UTF16PtrFromString(path)
		if e != nil {
			return e
		}
		sa := windows.SecurityAttributes{Length: uint32(unsafe.Sizeof(windows.SecurityAttributes{})), SecurityDescriptor: sd}
		if e = windows.CreateDirectory(p, &sa); e != nil {
			return e
		}
		h, err = openPath(path)
	}
	if err != nil {
		return err
	}
	defer windows.CloseHandle(h)
	var info windows.ByHandleFileInformation
	if err = windows.GetFileInformationByHandle(h, &info); err != nil {
		return err
	}
	if info.FileAttributes&windows.FILE_ATTRIBUTE_DIRECTORY == 0 {
		return errors.New("Windows service path is not a directory")
	}
	sd, err := windows.GetSecurityInfo(h, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION)
	if err != nil {
		return err
	}
	control, _, err := sd.Control()
	if err != nil {
		return err
	}
	if control&windows.SE_DACL_PROTECTED == 0 {
		return errors.New("Windows service directory must have a protected DACL")
	}
	return checkHandle(h, public, false)
}

func Check(path string, public bool) error {
	hs, err := ancestors(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer closeHandles(hs)
	h, err := openPath(path)
	if err != nil {
		return err
	}
	defer windows.CloseHandle(h)
	return checkHandle(h, public, false)
}

// Protect applies the exact policy only inside an already verified directory.
func Protect(path string, public bool) error {
	if err := Check(filepath.Dir(path), public); err != nil {
		return err
	}
	h, err := openPath(path)
	if err != nil {
		return err
	}
	defer windows.CloseHandle(h)
	sddl := PrivateSDDL
	if public {
		sddl = PublicSDDL
	}
	sd, err := windows.SecurityDescriptorFromString(sddl)
	if err != nil {
		return err
	}
	acl, _, err := sd.DACL()
	if err != nil {
		return err
	}
	owner, _, err := sd.Owner()
	if err != nil {
		return err
	}
	return windows.SetNamedSecurityInfo(path, windows.SE_FILE_OBJECT, windows.DACL_SECURITY_INFORMATION|windows.OWNER_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION, owner, nil, acl, nil)
}

func RemoveFile(path string) error {
	if err := Check(path, true); errors.Is(err, os.ErrNotExist) {
		return nil
	} else if err != nil {
		return err
	}
	if err := os.Remove(path); err == nil {
		return nil
	}
	// Loaded executables can be renamed on Windows, then removed at reboot.
	nonce := make([]byte, 16)
	if _, err := rand.Read(nonce); err != nil {
		return err
	}
	target := path + ".delete-" + hex.EncodeToString(nonce)
	if err := os.Rename(path, target); err != nil {
		return err
	}
	p, err := windows.UTF16PtrFromString(target)
	if err != nil {
		return err
	}
	return windows.MoveFileEx(p, nil, windows.MOVEFILE_DELAY_UNTIL_REBOOT)
}

// CreatePrivateTemp creates download bytes admin-only even in the public cache.
func CreatePrivateTemp(dir, prefix string) (*os.File, error) {
	if err := Check(dir, true); err != nil {
		return nil, err
	}
	sd, err := windows.SecurityDescriptorFromString(PrivateSDDL)
	if err != nil {
		return nil, err
	}
	sa := windows.SecurityAttributes{Length: uint32(unsafe.Sizeof(windows.SecurityAttributes{})), SecurityDescriptor: sd}
	nonce := make([]byte, 16)
	if _, err = rand.Read(nonce); err != nil {
		return nil, err
	}
	path := filepath.Join(dir, prefix+hex.EncodeToString(nonce))
	p, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return nil, err
	}
	h, err := windows.CreateFile(p, windows.GENERIC_READ|windows.GENERIC_WRITE, windows.FILE_SHARE_READ|windows.FILE_SHARE_WRITE, &sa, windows.CREATE_NEW, windows.FILE_ATTRIBUTE_NORMAL, 0)
	if err != nil {
		return nil, err
	}
	return os.NewFile(uintptr(h), path), nil
}

func RemoveDirectory(path string) error {
	if err := Check(path, true); errors.Is(err, os.ErrNotExist) {
		return nil
	} else if err != nil {
		return err
	}
	err := os.Remove(path)
	if !errors.Is(err, windows.ERROR_DIR_NOT_EMPTY) {
		return err
	}
	// Loaded files already scheduled above disappear first at reboot. Windows
	// removes this directory only if it is empty at that point.
	p, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	return windows.MoveFileEx(p, nil, windows.MOVEFILE_DELAY_UNTIL_REBOOT)
}
