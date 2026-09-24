//! 全域日志：每个可观察点都落一条 —— 环形缓冲（内存）+ logcat 镜像。
//! 通过 HTTP /logs 端点可整段拉回（这是"改→测→读日志"闭环的取数口）。

use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const RING_CAP: usize = 1500;

#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    /// epoch 毫秒
    pub ts: u64,
    /// I / W / E
    pub level: String,
    pub msg: String,
}

static RING: Lazy<Mutex<VecDeque<LogEntry>>> = Lazy::new(|| Mutex::new(VecDeque::with_capacity(RING_CAP)));

/// 记录一条日志：进环形缓冲 + 镜像到 log crate（logcat tag=chonggou）。
pub fn record(level: &str, msg: &str) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let entry = LogEntry {
        ts,
        level: level.to_owned(),
        msg: msg.to_owned(),
    };
    {
        let mut q = RING.lock().unwrap();
        q.push_back(entry);
        while q.len() > RING_CAP {
            q.pop_front();
        }
    }
    let lv = match level {
        "E" => log::Level::Error,
        "W" => log::Level::Warn,
        _ => log::Level::Info,
    };
    log::log!(lv, "{}", msg);
}

/// 拉取最近 n 条（时间升序）。
pub fn recent(n: usize) -> Vec<LogEntry> {
    let q = RING.lock().unwrap();
    let skip = q.len().saturating_sub(n);
    q.iter().skip(skip).cloned().collect()
}

/// 当前缓冲条数。
pub fn len() -> usize {
    RING.lock().unwrap().len()
}

#[macro_export]
macro_rules! chlog {
    (info, $($arg:tt)+) => { $crate::logging::record("I", &format!($($arg)+)) };
    (warn, $($arg:tt)+) => { $crate::logging::record("W", &format!($($arg)+)) };
    (error, $($arg:tt)+) => { $crate::logging::record("E", &format!($($arg)+)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_latest() {
        for i in 0..(RING_CAP + 100) {
            record("I", &format!("t{i}"));
        }
        let all = recent(RING_CAP);
        assert_eq!(all.len(), RING_CAP);
        assert_eq!(all.last().unwrap().msg, format!("t{}", RING_CAP + 99));
        assert_eq!(all.first().unwrap().msg, format!("t100"));
    }

    #[test]
    fn recent_n_smaller() {
        record("I", "marker-x");
        let last5 = recent(5);
        assert!(last5.len() <= 5);
        assert!(last5.iter().any(|e| e.msg == "marker-x"));
    }
}
