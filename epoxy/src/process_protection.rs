//! Process-local protection for credentials handled by the helper.
//!
//! Applied before the runtime, GUI, native modules and request handlers start.
//! This does not revoke an already attached tracer or protect against privileged
//! processes or a compromised launcher. Normal application diagnostics still work.
pub fn enable() -> std::io::Result<()> {
    // PR_SET_DUMPABLE changes only this process. It disables ordinary core dumps
    // and unprivileged ptrace-style access, including process_vm_readv.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
