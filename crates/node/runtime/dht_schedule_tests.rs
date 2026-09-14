use super::DhtRefreshSchedule;

#[test]
fn startup_retries_fit_inside_the_sixty_second_acceptance_window() {
    let mut schedule = DhtRefreshSchedule::new(300);
    let mut intervals = vec![schedule.current_interval_secs()];
    for _ in 0..6 {
        schedule.record_refresh();
        intervals.push(schedule.current_interval_secs());
    }

    assert_eq!(intervals, [5, 5, 10, 15, 30, 60, 300]);
    assert_eq!(intervals[..4].iter().sum::<u64>(), 35);
}
