use nix::errno::errno;
use nix::libc::{kill, ESRCH};
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::Pid;
use std::collections::HashSet;
use std::fs::read_dir;
use std::io::{Error, ErrorKind};
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, SystemTime};

use crate::file_util;

pub fn process_state(pid: Pid) -> Result<char, Error> {
    if pid == Pid::from_raw(0) || pid == nix::unistd::getpid() {
        return Ok('R');
    }

    let proc_file = format!("/proc/{:?}/stat", pid.as_raw());
    let stat_path = Path::new(&proc_file);
    let first_line = file_util::read_first_line(stat_path)?;
    let stat: Vec<String> = first_line
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    if stat.len() < 3 {
        return Err(Error::new(
            ErrorKind::Other,
            format!("invalid process stat format"),
        ));
    }

    let p_stat: Vec<char> = stat[3].trim().chars().collect();

    if p_stat.len() == 0 {
        return Err(Error::new(
            ErrorKind::Other,
            format!("invalid process state"),
        ));
    }
    return Ok(p_stat[0]);
}

pub fn alive(pid: Pid) -> bool {
    if pid < Pid::from_raw(0) {
        return false;
    }

    if pid <= Pid::from_raw(1) {
        return true;
    }

    if pid == nix::unistd::getpid() {
        return true;
    }

    let ret = process_state(pid);
    if ret.is_err() {
        return false;
    }
    if ret.unwrap() == 'Z' {
        return false;
    }

    true
}

pub fn valid_pid(pid: Pid) -> bool {
    if pid <= Pid::from_raw(0) {
        return false;
    }

    true
}

pub fn kill_all_pids(signal: i32) -> HashSet<i32> {
    let mut pids: HashSet<i32> = HashSet::new();
    let proc_path = Path::new("/proc");
    let read_dir = read_dir(proc_path).unwrap();
    for entry in read_dir {
        if let Ok(dir_entry) = entry {
            // Skip files.
            if let Ok(file_type) = dir_entry.file_type() {
                if file_type.is_file() {
                    continue;
                }
            }
            let file_name = String::from(dir_entry.file_name().to_str().unwrap());
            // Check pid directory.
            if let Ok(pid_raw) = file_name.parse::<i32>() {
                unsafe {
                    log::debug!("killing pid: {} by signal {}", pid_raw, signal);
                    kill(pid_raw, signal);
                    pids.insert(pid_raw);
                }
            } else {
                continue;
            }
        }
    }
    pids
}

pub fn wait_pids(mut pids: HashSet<i32>, timeout: u64) -> HashSet<i32> {
    let now = SystemTime::now();
    let until = now + Duration::from_micros(timeout);

    // remove PID1, we shouldn't wait our self.
    pids.remove(&1);

    loop {
        // 1. Find killed process by kernel.
        loop {
            if let Ok(wait_status) = waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                if let Some(pid) = wait_status.pid() {
                    log::debug!("successfully killed pid: {} found by kernel.", pid.as_raw());
                    pids.remove(&pid.as_raw());
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        // 2. Find killed process by sending sig: 0.
        let mut removed_pids: HashSet<i32> = HashSet::new();
        for pid in &pids {
            unsafe {
                let res = kill(*pid, 0);
                if res == 0 || errno() != ESRCH {
                    continue;
                }
                removed_pids.insert(*pid);
            }
        }
        for pid in removed_pids {
            log::debug!("successfully killed pid: {} found by ourself.", pid);
            pids.remove(&pid);
        }
        // 3. Sleep 1s to wait pid exits.
        sleep(Duration::from_secs(1));
        // 4. Wait or give up.
        if pids.is_empty() {
            break;
        }
        if SystemTime::now() >= until {
            log::info!("some pids haven't been killed yet, stop waiting.");
            break;
        }
    }
    pids
}
