//! The machine a session runs on: which cores inference should use, and
//! whether memory is tight enough to give up speed for it.

use std::sync::OnceLock;

/// Below this much memory, CPU sessions allocate each tensor as it is needed
/// instead of keeping an arena and a precomputed memory plan. That is slower,
/// but on a 4 GB Orange Pi it decides whether an image finishes at all.
const LOW_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;

pub fn low_memory() -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let total = system.total_memory();
    total > 0 && total < LOW_MEMORY_BYTES
}

/// The fast cores this process now runs on, when its CPU mixes fast and slow
/// cores (big.LITTLE, such as the RK3588's four Cortex-A76 and four Cortex-A55
/// cores). Inference split evenly across all of them waits on the slow ones,
/// and an Orange Pi 5 Pro restarted under that load. The first call keeps this
/// process to those cores, so threads started afterwards (the runtime's thread
/// pool among them) stay there; inference then uses one thread per fast core.
/// An affinity set by the caller is only ever narrowed. None when every
/// allowed core is alike, or off Linux.
pub fn fast_cores() -> Option<&'static [usize]> {
    static FAST: OnceLock<Option<Vec<usize>>> = OnceLock::new();
    FAST.get_or_init(keep_to_fast_cores).as_deref()
}

#[cfg(target_os = "linux")]
fn keep_to_fast_cores() -> Option<Vec<usize>> {
    let capacities: Vec<(usize, u64)> = linux::allowed_cpus()?
        .into_iter()
        .filter_map(|cpu| linux::capacity(cpu).map(|capacity| (cpu, capacity)))
        .collect();
    let fast = fastest(&capacities)?;
    linux::set_affinity(&fast).ok()?;
    Some(fast)
}

#[cfg(not(target_os = "linux"))]
fn keep_to_fast_cores() -> Option<Vec<usize>> {
    None
}

/// The fast cores, when some cores are much slower: those with at least three
/// quarters of the top capacity. Clusters of the same fast core can differ a
/// little (an RK3588's Cortex-A76 pairs report 1003 and 1024), while its
/// Cortex-A55 cores report about 405.
#[cfg(any(target_os = "linux", test))]
fn fastest(capacities: &[(usize, u64)]) -> Option<Vec<usize>> {
    let max = capacities.iter().map(|&(_, capacity)| capacity).max()?;
    let fast: Vec<usize> = capacities
        .iter()
        .filter(|&&(_, capacity)| capacity * 4 >= max * 3)
        .map(|&(cpu, _)| cpu)
        .collect();
    (fast.len() < capacities.len()).then_some(fast)
}

/// Cores as the kernel numbers them, such as "4-7".
pub fn describe(cores: &[usize]) -> String {
    match (cores.first(), cores.last()) {
        (Some(first), Some(last)) if last - first + 1 == cores.len() && cores.len() > 1 => {
            format!("{first}-{last}")
        }
        _ => cores
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(","),
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{fs, io, mem};

    /// The kernel's relative core speed, or the core's top frequency.
    pub fn capacity(cpu: usize) -> Option<u64> {
        let base = format!("/sys/devices/system/cpu/cpu{cpu}");
        number(&format!("{base}/cpu_capacity"))
            .or_else(|| number(&format!("{base}/cpufreq/cpuinfo_max_freq")))
    }

    fn number(path: &str) -> Option<u64> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    pub fn allowed_cpus() -> Option<Vec<usize>> {
        // SAFETY: cpu_set_t is plain data; the kernel fills it in.
        unsafe {
            let mut set: libc::cpu_set_t = mem::zeroed();
            if libc::sched_getaffinity(0, mem::size_of::<libc::cpu_set_t>(), &mut set) != 0 {
                return None;
            }
            Some(
                (0..libc::CPU_SETSIZE as usize)
                    .filter(|&cpu| libc::CPU_ISSET(cpu, &set))
                    .collect(),
            )
        }
    }

    pub fn set_affinity(cpus: &[usize]) -> io::Result<()> {
        // SAFETY: cpu_set_t is plain data, filled here and read by the kernel.
        unsafe {
            let mut set: libc::cpu_set_t = mem::zeroed();
            for &cpu in cpus {
                libc::CPU_SET(cpu, &mut set);
            }
            if libc::sched_setaffinity(0, mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_fast_cores_of_a_mixed_cpu() {
        // An Orange Pi 5 Pro's RK3588S: Cortex-A55 cores 0-3, then two
        // Cortex-A76 pairs whose top speeds differ slightly.
        let capacities = [405, 405, 405, 405, 1003, 1003, 1024, 1024];
        let rk3588: Vec<(usize, u64)> = capacities.into_iter().enumerate().collect();
        assert_eq!(fastest(&rk3588), Some(vec![4, 5, 6, 7]));
        assert_eq!(describe(&[4, 5, 6, 7]), "4-7");
        assert_eq!(describe(&[1, 3]), "1,3");
    }

    #[test]
    fn leaves_cores_alone_when_they_are_alike() {
        let alike: Vec<(usize, u64)> = (0..8).map(|cpu| (cpu, 1024)).collect();
        assert_eq!(fastest(&alike), None);
        assert_eq!(fastest(&[]), None);
    }
}
