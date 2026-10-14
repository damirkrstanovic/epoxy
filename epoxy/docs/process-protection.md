# Process memory protection

Epoxy calls Linux `prctl(PR_SET_DUMPABLE, 0)` before creating the async runtime,
parsing configuration, starting logging, loading native modules or accepting
requests. Failure aborts startup with a fixed contextual error. There is no
runtime or environment opt-out. This also applies to short-lived CLI commands.

This prevents ordinary unprivileged ptrace-style access and ordinary core dumps
for this process. Masked application diagnostics and tray inspection continue to
work. It does not change system sysctls, other processes or the user's shell.

It does not revoke a debugger that attached before this call, defeat a process
with the relevant tracing capability, or protect against a modified binary,
malicious loader/launcher, browser, compositor or native module. It does not lock
all memory against swapping or erase credentials. See the separate PIN memory
feature for credential-buffer cleanup.

The kernel regression test starts synthetic child processes. Its unprotected
control permits a parent memory read; the protected child reports dumpable=0 and
rejects the same read with EPERM. No real Epoxy process, card or PIN is inspected.
The baseline regression failed before the implementation was enabled.

Source: [Linux PR_SET_DUMPABLE](https://man7.org/linux/man-pages/man2/PR_SET_DUMPABLE.2const.html).
