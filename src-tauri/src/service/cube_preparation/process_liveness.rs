#[cfg(windows)]
pub(crate) fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    const EXIT_CODE_STILL_ACTIVE: u32 = 259;
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    let mut exit_code = 0u32;
    let known = unsafe { GetExitCodeProcess(handle, &mut exit_code) } != 0;
    unsafe { CloseHandle(handle) };
    !known || exit_code == EXIT_CODE_STILL_ACTIVE
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn process_exists(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    if pid <= 0 {
        return true;
    }
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(crate) fn process_exists(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::process_exists;

    #[test]
    fn the_running_process_exists_and_a_finished_child_does_not() {
        assert!(process_exists(std::process::id()));
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(process_exists(pid) || child.try_wait().unwrap().is_some());
        child.wait().unwrap();
        drop(child);
        assert!(!process_exists(pid), "process {pid} has exited");
    }
}
