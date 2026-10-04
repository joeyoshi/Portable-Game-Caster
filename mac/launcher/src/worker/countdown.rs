use std::sync::atomic::{
    AtomicBool,
    Ordering,
};
use std::thread;
use std::time::{
    Duration,
    Instant,
};


// -----------------------------------------------------------------------------
// Visible countdowns
//
// The displayed value is derived from the monotonic deadline and is emitted
// when a whole-second boundary is crossed, independently of how long the
// worker's own probe/retry iterations take.
// -----------------------------------------------------------------------------

fn whole_seconds(
    remaining: Duration,
) -> u64 {
    remaining
        .as_millis()
        .div_ceil(1000) as u64
}


// Human-readable ceiling countdown.
pub fn seconds_remaining(
    deadline: Instant,
    now: Instant,
    maximum: u8,
) -> u8 {
    whole_seconds(
        deadline.saturating_duration_since(now)
    )
    .min(maximum as u64) as u8
}


// Blocks until `deadline` passes or `stop` is set, calling `on_tick` with the
// whole seconds remaining once `starts_at` has been reached and again each time
// that value changes.
//
// `on_tick` may be called more than once with the same value (for example after
// the owning thread is unparked), so callers de-duplicate.
pub fn run(
    starts_at: Instant,
    deadline: Instant,
    maximum: u8,
    stop: &AtomicBool,
    mut on_tick: impl FnMut(u8),
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }


        let now =
            Instant::now();


        if now < starts_at {
            thread::park_timeout(
                starts_at - now
            );

            continue;
        }


        let remaining =
            deadline.saturating_duration_since(now);


        if remaining.is_zero() {
            return;
        }


        on_tick(
            seconds_remaining(
                deadline,
                now,
                maximum,
            )
        );


        // Sleep until the displayed value is due to change.
        let next_boundary =
            Duration::from_secs(
                whole_seconds(remaining) - 1
            );


        thread::park_timeout(
            remaining - next_boundary
        );
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn seconds_round_up() {
        let now =
            Instant::now();

        let at =
            |millis| {
                seconds_remaining(
                    now + Duration::from_millis(millis),
                    now,
                    15,
                )
            };

        assert_eq!(at(0), 0);
        assert_eq!(at(1), 1);
        assert_eq!(at(1000), 1);
        assert_eq!(at(1001), 2);
        assert_eq!(at(14_999), 15);
        assert_eq!(at(15_000), 15);
        assert_eq!(at(25_000), 15);
    }


    #[test]
    fn ticks_follow_real_seconds() {
        let started =
            Instant::now();

        let deadline =
            started + Duration::from_millis(3500);

        let stop =
            AtomicBool::new(false);

        let mut ticks:
            Vec<(u8, Duration)> =
                Vec::new();

        run(
            started,
            deadline,
            15,
            &stop,
            |seconds| {
                if ticks.last().map(|tick| tick.0) != Some(seconds) {
                    ticks.push((seconds, started.elapsed()));
                }
            },
        );

        let values:
            Vec<u8> =
                ticks.iter().map(|tick| tick.0).collect();

        assert_eq!(values, vec![4, 3, 2, 1]);

        // 4 is shown immediately; 3, 2, 1 at 0.5s, 1.5s, 2.5s.
        let expected = [0_u64, 500, 1500, 2500];

        for (tick, expected) in ticks.iter().zip(expected) {
            let late_by =
                tick.1.saturating_sub(Duration::from_millis(expected));

            assert!(tick.1 >= Duration::from_millis(expected));
            assert!(late_by < Duration::from_millis(150), "{ticks:?}");
        }

        assert!(started.elapsed() >= Duration::from_millis(3500));
    }


    #[test]
    fn waits_for_start_and_stops_on_request() {
        let started =
            Instant::now();

        let stop =
            AtomicBool::new(false);

        let mut first_tick =
            None;

        run(
            started + Duration::from_millis(300),
            started + Duration::from_secs(10),
            15,
            &stop,
            |seconds| {
                first_tick =
                    Some((seconds, started.elapsed()));

                stop.store(true, Ordering::Relaxed);
            },
        );

        let (seconds, at) =
            first_tick.expect("countdown should tick once");

        assert_eq!(seconds, 10);
        assert!(at >= Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
