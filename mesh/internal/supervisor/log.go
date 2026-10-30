package supervisor

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

const logLimit = 10 * 1024 * 1024
const tailLimit = 16 * 1024

type rotatingLog struct {
	mu     sync.Mutex
	path   string
	file   *os.File
	size   int64
	recent []byte
}

func openLog(dir, name string) (*rotatingLog, error) {
	dir, err := secureDirectory(dir, 0700, nil)
	if err != nil {
		return nil, err
	}
	info, err := os.Stat(dir)
	if err != nil {
		return nil, err
	}
	if info.Mode().Perm() != 0700 {
		return nil, errors.New("worker log directory must be private (0700)")
	}
	path := filepath.Join(dir, name+".log")
	file, err := openLogFile(path)
	if err != nil {
		return nil, err
	}
	info, err = file.Stat()
	if err != nil {
		file.Close()
		return nil, err
	}
	start := max(int64(0), info.Size()-tailLimit)
	tail := make([]byte, info.Size()-start)
	if _, err = file.ReadAt(tail, start); err != nil && err != io.EOF {
		file.Close()
		return nil, err
	}
	return &rotatingLog{path: path, file: file, size: info.Size(), recent: tail}, nil
}
func openLogFile(path string) (*os.File, error) {
	if info, err := os.Lstat(path); err == nil {
		if !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
			return nil, fmt.Errorf("unsafe worker log: %s", path)
		}
		if err = trustedOwner(info); err != nil {
			return nil, err
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}
	return os.OpenFile(path, os.O_CREATE|os.O_RDWR|os.O_APPEND, 0600)
}
func (l *rotatingLog) Write(p []byte) (int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	written := 0
	for len(p) > 0 {
		if l.size >= logLimit {
			if err := l.rotate(); err != nil {
				return written, err
			}
		}
		n, err := l.file.Write(p[:min(int64(len(p)), logLimit-l.size)])
		l.size += int64(n)
		written += n
		l.recent = append(l.recent, p[:n]...)
		if len(l.recent) > tailLimit {
			copy(l.recent, l.recent[len(l.recent)-tailLimit:])
			l.recent = l.recent[:tailLimit]
		}
		p = p[n:]
		if err != nil {
			return written, err
		}
	}
	return written, nil
}
func (l *rotatingLog) rotate() error {
	if err := l.file.Close(); err != nil {
		return err
	}
	if err := os.Remove(l.path + ".2"); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	if err := os.Rename(l.path+".1", l.path+".2"); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	if err := os.Rename(l.path, l.path+".1"); err != nil {
		return err
	}
	file, err := openLogFile(l.path)
	if err != nil {
		return err
	}
	l.file = file
	l.size = 0
	return nil
}
func (l *rotatingLog) tail() string {
	l.mu.Lock()
	defer l.mu.Unlock()
	return strings.ToValidUTF8(string(l.recent), "�")
}
func (l *rotatingLog) Close() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.file == nil {
		return nil
	}
	err := l.file.Close()
	l.file = nil
	return err
}
