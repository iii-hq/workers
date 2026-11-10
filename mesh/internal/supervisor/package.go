package supervisor

import (
	"archive/tar"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
)

// The largest published worker package is one file of about 100 MB.
const (
	maxPackageEntries = 1024
	maxPackageBytes   = 1 << 30
)

// executable returns what to run for a digest-verified cached artifact: the
// artifact itself, or the worker binary of the gzip tar package it holds.
// The registry describes an artifact by URL and SHA256 only, so the format is
// read from the content: no native executable or script starts with the gzip magic.
// A package is extracted once, beside the artifact, and compared with the
// artifact again before every later execution.
func executable(artifact, worker string) (string, error) {
	file, err := os.Open(artifact)
	if err != nil {
		return "", err
	}
	defer file.Close()
	magic := make([]byte, 2)
	if _, err = io.ReadFull(file, magic); err != nil || magic[0] != 0x1f || magic[1] != 0x8b {
		return artifact, nil
	}
	if _, err = file.Seek(0, io.SeekStart); err != nil {
		return "", err
	}
	dir := artifact + ".d"
	info, err := os.Lstat(dir)
	fresh := errors.Is(err, os.ErrNotExist)
	root := dir
	switch {
	case fresh:
		// Extract beside the final path so the rename below cannot cross a file system.
		if root, err = os.MkdirTemp(filepath.Dir(artifact), ".extract-*"); err != nil {
			return "", err
		}
		defer os.RemoveAll(root)
	case err != nil:
		return "", err
	case !info.IsDir():
		return "", fmt.Errorf("unsafe cached worker package: %s", dir)
	}
	binary, err := Unpack(file, root, worker, fresh)
	if err != nil {
		return "", fmt.Errorf("worker package: %w", err)
	}
	if fresh {
		// The package appears complete or not at all.
		if err = os.Rename(root, dir); err != nil {
			return "", err
		}
	}
	return filepath.Join(dir, binary), nil
}

// Unpack extracts a gzip tar into the empty directory root. When fresh is false
// it instead checks that root holds exactly the files of the archive. It returns
// the root-level executable to run: the one named after the worker, or the only one.
func Unpack(archive io.Reader, root, worker string, fresh bool) (string, error) {
	if runtime.GOOS == "windows" {
		// shortcut: gzip tar packages are POSIX-only because the registry ships Windows workers as .zip, add zip extraction with winsecurity protection when Windows nodes run registry workers.
		return "", errors.New("gzip tar is not supported on Windows")
	}
	compressed, err := gzip.NewReader(archive)
	if err != nil {
		return "", err
	}
	reader := tar.NewReader(compressed)
	var executables []string
	var total int64
	files := 0
	for entries := 0; ; entries++ {
		header, err := reader.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return "", err
		}
		if entries == maxPackageEntries {
			return "", fmt.Errorf("more than %d entries", maxPackageEntries)
		}
		if header.Typeflag == tar.TypeXGlobalHeader {
			continue
		}
		if !filepath.IsLocal(header.Name) || slices.Contains(strings.Split(header.Name, "/"), "..") {
			return "", fmt.Errorf("unsafe path %q", header.Name)
		}
		name := filepath.Clean(header.Name)
		path := filepath.Join(root, name)
		switch header.Typeflag {
		case tar.TypeDir:
			if fresh {
				err = os.MkdirAll(path, 0755)
			}
		case tar.TypeReg:
			// The reader yields at most the declared size, so this bounds the bytes written.
			if header.Size > maxPackageBytes-total {
				return "", fmt.Errorf("more than %d bytes", maxPackageBytes)
			}
			total += header.Size
			files++
			mode := os.FileMode(0444)
			if header.Mode&0111 != 0 {
				mode = 0555
				if !strings.ContainsRune(name, filepath.Separator) {
					executables = append(executables, name)
				}
			}
			if fresh {
				err = writePackageFile(path, reader, mode)
			} else {
				digest := sha256.New()
				if _, err = io.Copy(digest, reader); err == nil {
					err = verifyArtifact(path, hex.EncodeToString(digest.Sum(nil)), mode)
				}
			}
		default:
			// Links and devices could reach outside the package.
			err = fmt.Errorf("unsupported entry %q of type %q", header.Name, header.Typeflag)
		}
		if err != nil {
			return "", err
		}
	}
	err = filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.IsDir() {
			files--
			// WalkDir does not follow links, but the file checks above resolve every
			// directory on the path: a link anywhere would point them elsewhere.
			if !fresh && !entry.Type().IsRegular() {
				return fmt.Errorf("unsafe cached worker package: %s", path)
			}
			return nil
		}
		if fresh {
			// A private service umask must not hide the package from the operator.
			return os.Chmod(path, 0755)
		}
		info, err := entry.Info()
		if err == nil && info.Mode().Perm() != 0755 {
			err = fmt.Errorf("unsafe cached worker package: %s", path)
		}
		if err == nil {
			err = checkArtifact(path, info)
		}
		return err
	})
	if err != nil {
		return "", err
	}
	if !fresh && files != 0 {
		return "", errors.New("cached package holds files that are not in its archive")
	}
	if !slices.Contains(executables, worker) {
		if len(executables) != 1 {
			return "", fmt.Errorf("no single executable for worker %s", worker)
		}
		worker = executables[0]
	}
	return worker, nil
}

func writePackageFile(path string, content io.Reader, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	// O_EXCL keeps a repeated archive path from replacing an earlier entry.
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	defer file.Close()
	if _, err = io.Copy(file, content); err != nil {
		return err
	}
	if err = file.Chmod(mode); err != nil {
		return err
	}
	if err = file.Sync(); err != nil {
		return err
	}
	return file.Close()
}
