//! Peak memory of the current process, in MiB: (peak working set, peak commit).

#[cfg(windows)]
pub fn peak() -> (f64, f64) {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
        (
            c.PeakWorkingSetSize as f64 / 1048576.0,
            c.PeakPagefileUsage as f64 / 1048576.0,
        )
    }
}

/// Linux: VmHWM from /proc. Other platforms report NaN.
#[cfg(not(windows))]
pub fn peak() -> (f64, f64) {
    let hwm = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map(|kb| kb / 1024.0)
        .unwrap_or(f64::NAN);
    (hwm, f64::NAN)
}
