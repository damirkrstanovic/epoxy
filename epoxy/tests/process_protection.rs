//! Kernel checks against a child containing synthetic data, never a real helper.
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

#[test]
fn process_probe() {
    let Ok(mode) = std::env::var("EPOXY_SYNTHETIC_DUMP_TEST") else {
        return;
    };
    if mode == "protected" {
        epoxy::process_protection::enable().unwrap();
    }
    let sentinel = Box::new(*b"synthetic-secret");
    let dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE) };
    println!(
        "PROBE {} {} {}",
        std::process::id(),
        sentinel.as_ptr() as usize,
        dumpable
    );
    std::io::stdout().flush().unwrap();
    let mut end = [0];
    let _ = std::io::stdin().read(&mut end);
    std::hint::black_box(sentinel);
}

#[test]
fn protection_blocks_parent_memory_read_without_changing_other_processes() {
    let parent_before = unsafe { libc::prctl(libc::PR_GET_DUMPABLE) };
    for protected in [false, true] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_probe", "--nocapture"])
            .env(
                "EPOXY_SYNTHETIC_DUMP_TEST",
                if protected { "protected" } else { "baseline" },
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "child ended before probe"
            );
            if let Some(start) = line.find("PROBE ") {
                line = line[start..].to_string();
                break;
            }
        }
        let values: Vec<usize> = line
            .split_whitespace()
            .skip(1)
            .map(|v| v.parse().unwrap())
            .collect();
        let mut bytes = [0u8; 16];
        let local = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let remote = libc::iovec {
            iov_base: values[1] as *mut _,
            iov_len: bytes.len(),
        };
        let read = unsafe { libc::process_vm_readv(values[0] as i32, &local, 1, &remote, 1, 0) };
        let error = std::io::Error::last_os_error();
        child.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(child.wait().unwrap().success());
        if protected {
            assert_eq!(values[2], 0, "process must disable core dumping");
            assert_eq!(read, -1);
            assert_eq!(error.raw_os_error(), Some(libc::EPERM));
        } else {
            assert_eq!(
                read, 16,
                "baseline parent must be able to read its synthetic child: {error}"
            );
            assert_eq!(&bytes, b"synthetic-secret");
        }
    }
    assert_eq!(unsafe { libc::prctl(libc::PR_GET_DUMPABLE) }, parent_before);
}
