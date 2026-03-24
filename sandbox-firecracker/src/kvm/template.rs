use anyhow::{bail, Context, Result};

use crate::types::VmSnapshot;

pub struct Template {
    pub snapshot: VmSnapshot,
    pub memfd: i32,
    pub mem_size: usize,
}

impl Template {
    pub fn load(vmstate_path: &str, memfile_path: &str, mem_size_mb: usize) -> Result<Self> {
        let saved = super::vmstate::load_vmstate(vmstate_path)?;
        let mem_size = mem_size_mb * 1024 * 1024;
        let snapshot = saved.to_snapshot(mem_size);

        let mem_data = std::fs::read(memfile_path)
            .with_context(|| format!("failed to read memory file from {memfile_path}"))?;

        if mem_data.len() > mem_size {
            bail!(
                "memory file ({} bytes) exceeds configured mem_size ({} bytes)",
                mem_data.len(),
                mem_size
            );
        }

        let name = std::ffi::CString::new("iii-template")
            .context("failed to create memfd name")?;

        // SAFETY: memfd_create is a Linux syscall that returns a file descriptor.
        // MFD_CLOEXEC (0x0001) ensures the fd is closed on exec.
        let memfd = unsafe { libc::memfd_create(name.as_ptr(), 0x0001) };
        if memfd < 0 {
            bail!("memfd_create failed: {}", std::io::Error::last_os_error());
        }

        // SAFETY: memfd is a valid fd from memfd_create above.
        let ret = unsafe { libc::ftruncate(memfd, mem_size as libc::off_t) };
        if ret < 0 {
            unsafe { libc::close(memfd) };
            bail!("ftruncate failed: {}", std::io::Error::last_os_error());
        }

        // SAFETY: We mmap the memfd as MAP_SHARED to write the template memory.
        // The fd is valid and the size was set by ftruncate above.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                mem_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                memfd,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            unsafe { libc::close(memfd) };
            bail!("mmap MAP_SHARED failed: {}", std::io::Error::last_os_error());
        }

        // SAFETY: ptr is a valid mmap region of mem_size bytes. mem_data.len() <= mem_size
        // was checked above. We copy the template memory into the shared mapping.
        unsafe {
            std::ptr::copy_nonoverlapping(mem_data.as_ptr(), ptr as *mut u8, mem_data.len());
            libc::munmap(ptr, mem_size);
        }

        tracing::info!(
            memfd,
            mem_size_mb,
            mem_file_bytes = mem_data.len(),
            "template loaded"
        );

        Ok(Template {
            snapshot,
            memfd,
            mem_size,
        })
    }
}

impl Drop for Template {
    fn drop(&mut self) {
        if self.memfd >= 0 {
            // SAFETY: memfd was created by memfd_create in Template::load.
            // We close it exactly once in Drop.
            unsafe {
                libc::close(self.memfd);
            }
            self.memfd = -1;
        }
    }
}

// SAFETY: Template's memfd is an integer file descriptor. The mmap was
// already unmapped after copying data. Only the fd remains, which is
// safe to share across threads behind Arc.
unsafe impl Send for Template {}
unsafe impl Sync for Template {}
