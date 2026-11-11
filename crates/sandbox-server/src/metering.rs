//! Per-session metering for billing (PLAN §4.14, M5). Each session keeps the CPU time its
//! commands used, the memory it held, and how many commands ran. A command's memory is the
//! filesystem bytes the session held when the command finished, weighted by the command's
//! wall time, so the integral over a session is memory-time in byte-milliseconds.
//!
//! The arithmetic lives in pure functions so it can be tested without a server. The server
//! records one exec with `SessionMeter::record_exec`, and the usage export reports each
//! session's meter.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// What one session has used. All fields only grow, so adding two meters gives the total.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct SessionMeter {
    /// Commands that finished, including those that failed.
    pub execs: u64,
    /// CPU time the session's commands used, in milliseconds.
    pub cpu_ms: u64,
    /// Wall time the session's commands ran, in milliseconds.
    pub wall_ms: u64,
    /// Memory-time: filesystem bytes held, times each command's wall time, in byte-milliseconds.
    pub memory_byte_ms: u64,
    /// The most filesystem bytes the session held at the end of any command.
    pub peak_memory_bytes: u64,
}

impl SessionMeter {
    /// Records one finished command. `memory_bytes` is the filesystem size when it finished.
    pub fn record_exec(&mut self, cpu_ms: u64, wall_ms: u64, memory_bytes: u64) {
        self.execs = self.execs.saturating_add(1);
        self.cpu_ms = self.cpu_ms.saturating_add(cpu_ms);
        self.wall_ms = self.wall_ms.saturating_add(wall_ms);
        self.memory_byte_ms = self
            .memory_byte_ms
            .saturating_add(memory_byte_ms(memory_bytes, wall_ms));
        self.peak_memory_bytes = self.peak_memory_bytes.max(memory_bytes);
    }

    /// Adds another meter's totals to this one, as a billing period's total is the sum of its
    /// parts. The peak is the larger of the two peaks, not their sum.
    pub fn add(&mut self, other: &SessionMeter) {
        self.execs = self.execs.saturating_add(other.execs);
        self.cpu_ms = self.cpu_ms.saturating_add(other.cpu_ms);
        self.wall_ms = self.wall_ms.saturating_add(other.wall_ms);
        self.memory_byte_ms = self.memory_byte_ms.saturating_add(other.memory_byte_ms);
        self.peak_memory_bytes = self.peak_memory_bytes.max(other.peak_memory_bytes);
    }
}

/// Memory-time for one command: bytes held times the command's wall time, saturating.
pub fn memory_byte_ms(memory_bytes: u64, wall_ms: u64) -> u64 {
    memory_bytes.saturating_mul(wall_ms)
}

/// The meters of every session, keyed by session ID, with the owning key of each.
#[derive(Clone, Default)]
pub struct MeterTable(Arc<Mutex<HashMap<String, (String, SessionMeter)>>>);

impl MeterTable {
    /// Opens a meter for a new session, so it is reported even before any command runs.
    pub fn open(&self, owner: &str, session: &str) {
        let mut table = self.lock();
        table
            .entry(session.to_string())
            .or_insert_with(|| (owner.to_string(), SessionMeter::default()));
    }

    /// Records one finished command against a session's meter.
    pub fn record_exec(&self, session: &str, cpu_ms: u64, wall_ms: u64, memory_bytes: u64) {
        let mut table = self.lock();
        if let Some((_, meter)) = table.get_mut(session) {
            meter.record_exec(cpu_ms, wall_ms, memory_bytes);
        }
    }

    /// Every session the key owns, with its meter, ordered by session ID.
    pub fn for_owner(&self, owner: &str) -> Vec<(String, SessionMeter)> {
        let table = self.lock();
        let mut sessions: Vec<(String, SessionMeter)> = table
            .iter()
            .filter(|(_, (o, _))| o == owner)
            .map(|(id, (_, meter))| (id.clone(), meter.clone()))
            .collect();
        sessions.sort_by(|a, b| a.0.cmp(&b.0));
        sessions
    }

    /// The meters of every session the key owns, added together.
    pub fn total_for_owner(&self, owner: &str) -> SessionMeter {
        self.for_owner(owner)
            .iter()
            .fold(SessionMeter::default(), |mut total, (_, meter)| {
                total.add(meter);
                total
            })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (String, SessionMeter)>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_exec_adds_its_cpu_wall_and_memory_time() {
        let mut meter = SessionMeter::default();
        meter.record_exec(12, 40, 1_000);
        assert_eq!(meter.execs, 1);
        assert_eq!(meter.cpu_ms, 12);
        assert_eq!(meter.wall_ms, 40);
        assert_eq!(meter.memory_byte_ms, 40_000);
        assert_eq!(meter.peak_memory_bytes, 1_000);
    }

    #[test]
    fn execs_accumulate_and_the_peak_is_the_largest_size_seen() {
        let mut meter = SessionMeter::default();
        meter.record_exec(5, 10, 300);
        meter.record_exec(7, 20, 900);
        meter.record_exec(1, 5, 200);
        assert_eq!(meter.execs, 3);
        assert_eq!(meter.cpu_ms, 13);
        assert_eq!(meter.wall_ms, 35);
        assert_eq!(meter.memory_byte_ms, 300 * 10 + 900 * 20 + 200 * 5);
        assert_eq!(meter.peak_memory_bytes, 900);
    }

    #[test]
    fn a_command_that_takes_no_time_holds_memory_for_none() {
        assert_eq!(memory_byte_ms(10_000, 0), 0);
    }

    #[test]
    fn memory_time_saturates_rather_than_wrapping() {
        assert_eq!(memory_byte_ms(u64::MAX, 2), u64::MAX);
        let mut meter = SessionMeter {
            cpu_ms: u64::MAX - 1,
            ..SessionMeter::default()
        };
        meter.record_exec(10, 0, 0);
        assert_eq!(meter.cpu_ms, u64::MAX);
    }

    #[test]
    fn adding_meters_sums_the_totals_and_keeps_the_larger_peak() {
        let mut a = SessionMeter::default();
        a.record_exec(3, 4, 500);
        let mut b = SessionMeter::default();
        b.record_exec(6, 8, 200);
        let mut total = a.clone();
        total.add(&b);
        assert_eq!(total.execs, 2);
        assert_eq!(total.cpu_ms, 9);
        assert_eq!(total.wall_ms, 12);
        assert_eq!(total.memory_byte_ms, 500 * 4 + 200 * 8);
        assert_eq!(total.peak_memory_bytes, 500);
    }

    #[test]
    fn a_new_session_is_reported_with_a_zero_meter() {
        let table = MeterTable::default();
        table.open("key-a", "s1");
        table.open("key-b", "s2");
        let mine = table.for_owner("key-a");
        assert_eq!(mine, vec![("s1".to_string(), SessionMeter::default())]);
    }

    #[test]
    fn exec_records_land_on_the_session_and_only_its_owner_sees_them() {
        let table = MeterTable::default();
        table.open("key-a", "s1");
        table.open("key-b", "s2");
        table.record_exec("s1", 9, 20, 4_096);
        let mine = table.for_owner("key-a");
        assert_eq!(mine[0].1.cpu_ms, 9);
        assert_eq!(mine[0].1.memory_byte_ms, 4_096 * 20);
        assert_eq!(table.for_owner("key-b")[0].1.execs, 0);
    }

    #[test]
    fn an_exec_on_an_unknown_session_is_ignored() {
        let table = MeterTable::default();
        table.record_exec("nope", 1, 1, 1);
        assert!(table.for_owner("key-a").is_empty());
    }
}
