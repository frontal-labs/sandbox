//! Unit tests moved out of src/metering.rs.

use sandbox_server::metering::{memory_byte_ms, MeterTable, SessionMeter};

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

#[test]
fn closing_a_session_keeps_its_meter_and_closes_it_once() {
    let table = MeterTable::default();
    table.open("key-a", "s1");
    table.record_exec("s1", 2, 3, 4);
    let final_meter = table.close("s1").expect("the session was open");
    assert_eq!(final_meter.execs, 1);
    assert!(table.close("s1").is_none(), "a second stop reports nothing");
    assert_eq!(table.for_owner("key-a")[0].1.cpu_ms, 2);
}

#[test]
fn a_restored_meter_keeps_its_totals_and_open_state() {
    let table = MeterTable::default();
    let mut meter = SessionMeter::default();
    meter.record_exec(5, 6, 7);
    table.restore("key-a", "s1", meter.clone(), true);
    assert_eq!(table.get("s1"), meter);
    assert!(table.close("s1").is_some());
    table.restore("key-a", "s2", meter, false);
    assert!(
        table.close("s2").is_none(),
        "a restored stopped session stays stopped"
    );
}
