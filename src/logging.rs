//! 全域日志：每个可观察点都落一条 —— 环形缓冲（内存）+ logcat 镜像。
//! 通过 HTTP /logs 端点可整段拉回（这是"改→测→读日志"闭环的取数口）。
//!
//! 防闪退纪律：锁一律 poison 恢复（`unwrap_or_else(|p| p.into_inner())`），
//! 游戏进程里绝不 panic。

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
        let mut q = RING.lock().unwrap_or_else(|p| p.into_inner());
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
    let q = RING.lock().unwrap_or_else(|p| p.into_inner());
    let skip = q.len().saturating_sub(n);
    q.iter().skip(skip).cloned().collect()
}

/// 当前缓冲条数。
pub fn len() -> usize {
    RING.lock().unwrap_or_else(|p| p.into_inner()).len()
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

    /// ⚠️ 这条测试曾经是**假绿**：原断言 `assert_eq!(all.first(), "t100")`
    /// 硬编了缓冲起点，隐含假设「跑这条时 RING 是空的」。但 RING 是全局
    /// `Lazy<Mutex<VecDeque>>`，Rust 测试默认多线程并行，别的用例
    /// （`recent_n_smaller` 塞 `marker-x`，以及其它模块间接触发 chlog!）
    /// 会先往里写，起点就随执行顺序漂 —— 断言 t100 实际拿到 t101。
    ///
    /// 修法：**只断言与顺序无关的不变量**（封顶 + 最新一条不被淘汰），
    /// 不再断言首条是哪一条 —— 首条本质上取决于「跑之前别人塞了多少」，
    /// 那是测试写错了，不是实现错了。
    #[test]
    fn ring_keeps_latest() {
        for i in 0..(RING_CAP + 100) {
            record("I", &format!("t{i}"));
        }
        let all = recent(RING_CAP);

        // ① 封顶成立：无论并行用例塞了多少，环形缓冲永远不超过 RING_CAP
        assert!(len() <= RING_CAP, "缓冲超容: {}", len());
        assert!(all.len() <= RING_CAP, "recent 返回超量: {}", all.len());

        // ② 我们写的最新一条一定还在 —— 证明丢的是最旧的、不是最新的
        let newest = format!("t{}", RING_CAP + 99);
        assert!(
            all.iter().any(|e| e.msg == newest),
            "最新一条 {newest} 被淘汰了（丢错了头）"
        );

        // ③ 我们写的最后 50 条连续在位（尾部未被破坏）
        for i in (RING_CAP + 50)..=(RING_CAP + 99) {
            let want = format!("t{i}");
            assert!(all.iter().any(|e| e.msg == want), "尾部丢条目 {want}");
        }
    }

    #[test]
    fn recent_n_smaller() {
        record("I", "marker-x");
        let last5 = recent(5);
        assert!(last5.len() <= 5);
        assert!(last5.iter().any(|e| e.msg == "marker-x"));
    }

    /// 毒化锁真的造一个出来（子线程持锁时 panic），验证 record/recent 仍可用 ——
    /// 这条正是「游戏进程里绝不 panic」纪律的回归钉。
    ///
    /// ⚠️ 曾经的写法 `let _ = RING.lock();` 是 rustc 的 **deny(let_underscore_lock)**
    /// 硬错误，而且它根本没毒化锁（正常加解锁不会 poison）—— 名字骗人、什么也没测。
    /// 必须让持锁线程真的 panic，Mutex 才会被标 poisoned。
    #[test]
    fn survives_poisoned_lock() {
        let p = std::thread::spawn(|| {
            let _g = RING.lock();
            panic!("故意 panic 以毒化锁");
        });
        assert!(p.join().is_err(), "子线程应当 panic");

        // 现在 RING 已是 poisoned —— record/recent 仍必须正常工作（内部走 into_inner）
        record("I", "after-poison");
        assert!(recent(RING_CAP).iter().any(|e| e.msg == "after-poison"));
        assert!(len() > 0);
    }
}
