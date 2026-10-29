use super::*;

const STABLE: Duration = Duration::from_mins(10);
const HEALTHY: u32 = 35245;
const NEW: u32 = 37900;

fn at(minutes: i64) -> Timestamp {
    let start: Timestamp = "2026-10-28T02:00:00Z".parse().unwrap();
    start + SignedDuration::from_mins(minutes)
}

/// Steps through `inputs`, each a minute apart from `start`, and gathers what they said.
struct Run {
    state: Probation,
    verdicts: Vec<Verdict>,
}

impl Run {
    fn from(state: Probation) -> Self {
        Self {
            state,
            verdicts: Vec::new(),
        }
    }

    fn healthy() -> Self {
        Self::from(Probation {
            last_healthy: Some(HEALTHY),
            ..Probation::default()
        })
    }

    fn at(mut self, minute: i64, input: Input) -> Self {
        let (state, mut verdicts) = step(self.state, input, at(minute), STABLE);
        self.state = state;
        self.verdicts.append(&mut verdicts);
        self
    }
}

fn spawned(generation: u32, build: u32) -> Input {
    Input::Spawned { generation, build }
}

fn ended(generation: u32, end: End) -> Input {
    Input::Ended { generation, end }
}

#[test]
fn a_build_that_stays_up_ten_minutes_after_it_is_ready_passes() {
    let run = Run::healthy()
        .at(0, spawned(1, NEW))
        .at(1, Input::Ready { generation: 1 })
        .at(10, Input::Tick)
        .at(11, Input::Tick);

    assert_eq!(
        run.verdicts,
        [
            Verdict::Opened {
                build: NEW,
                from: Some(HEALTHY)
            },
            Verdict::Passed { build: NEW },
        ]
    );
    assert_eq!(run.state.last_healthy, Some(NEW));
    assert_eq!(run.state.open, None);
}

#[test]
fn the_window_counts_from_ready_not_from_the_spawn() {
    let run = Run::healthy()
        .at(0, spawned(1, NEW))
        .at(4, Input::Ready { generation: 1 })
        .at(12, Input::Tick);

    assert_eq!(passes_at(&run.state, STABLE), Some(at(14)));
    assert!(!run.verdicts.contains(&Verdict::Passed { build: NEW }));
    let run = run.at(14, Input::Tick);
    assert_eq!(run.verdicts.last(), Some(&Verdict::Passed { build: NEW }));
}

#[test]
fn degraded_counts_as_up() {
    // The artifacts task reports a degraded run as ready: it answers, if slowly.
    let run = Run::healthy()
        .at(0, spawned(1, NEW))
        .at(1, Input::Ready { generation: 1 })
        .at(2, Input::Ready { generation: 1 })
        .at(11, Input::Tick);

    assert_eq!(run.verdicts.last(), Some(&Verdict::Passed { build: NEW }));
}

#[test]
fn a_crash_before_ready_fails_it() {
    let run = Run::healthy()
        .at(0, spawned(1, NEW))
        .at(1, ended(1, End::Crashed));

    assert_eq!(
        run.verdicts.last(),
        Some(&Verdict::Failed {
            build: NEW,
            from: Some(HEALTHY),
            reason: Failure::Crashed
        })
    );
    assert_eq!(run.state.open, None);
    assert_eq!(run.state.last_healthy, Some(HEALTHY));
}

#[test]
fn a_hang_during_the_window_fails_it() {
    let run = Run::healthy()
        .at(0, spawned(1, NEW))
        .at(1, Input::Ready { generation: 1 })
        .at(6, ended(1, End::Hung))
        .at(20, Input::Tick);

    assert_eq!(
        run.verdicts.last(),
        Some(&Verdict::Failed {
            build: NEW,
            from: Some(HEALTHY),
            reason: Failure::Hung
        })
    );
    assert_eq!(run.state.last_healthy, Some(HEALTHY));
}

#[test]
fn a_bind_error_or_a_licence_refusal_leaves_it_open() {
    for end in [End::BindError, End::LicenceRejected] {
        let run = Run::healthy()
            .at(0, spawned(1, NEW))
            .at(1, Input::Ready { generation: 1 })
            .at(2, ended(1, end))
            .at(30, Input::Tick);

        assert_eq!(
            run.verdicts,
            [Verdict::Opened {
                build: NEW,
                from: Some(HEALTHY)
            }],
            "{end:?}"
        );
        assert_eq!(
            run.state.open,
            Some(OpenProbation {
                build: NEW,
                from: Some(HEALTHY)
            })
        );
    }
}

#[test]
fn a_requested_stop_leaves_it_open_and_the_next_run_starts_the_window_over() {
    let run = Run::healthy()
        .at(0, spawned(1, NEW))
        .at(1, Input::Ready { generation: 1 })
        .at(8, ended(1, End::Requested))
        .at(9, spawned(2, NEW))
        .at(12, Input::Tick);

    assert_eq!(run.state.run.map(|run| run.generation), Some(2));
    assert_eq!(passes_at(&run.state, STABLE), None, "not ready yet");
    let run = run.at(13, Input::Ready { generation: 2 }).at(23, Input::Tick);
    assert_eq!(
        run.verdicts,
        [
            Verdict::Opened {
                build: NEW,
                from: Some(HEALTHY)
            },
            Verdict::Passed { build: NEW },
        ]
    );
}

#[test]
fn a_run_of_the_last_healthy_build_opens_none() {
    let run = Run::healthy()
        .at(0, spawned(1, HEALTHY))
        .at(1, ended(1, End::Crashed));

    assert_eq!(run.verdicts, []);
    assert_eq!(run.state.open, None);
}

#[test]
fn a_run_of_another_build_closes_it_without_a_verdict() {
    let run = Run::healthy()
        .at(0, spawned(1, NEW))
        .at(1, ended(1, End::Requested))
        .at(2, spawned(2, HEALTHY))
        .at(3, ended(2, End::Crashed));

    assert_eq!(
        run.verdicts,
        [
            Verdict::Opened {
                build: NEW,
                from: Some(HEALTHY)
            },
            Verdict::Closed { build: NEW },
        ]
    );
    assert_eq!(run.state.last_healthy, Some(HEALTHY));
}

#[test]
fn an_older_generation_is_ignored() {
    let run = Run::healthy()
        .at(0, spawned(2, NEW))
        .at(1, Input::Ready { generation: 1 })
        .at(2, ended(1, End::Crashed))
        .at(12, Input::Tick);

    assert_eq!(
        run.verdicts,
        [Verdict::Opened {
            build: NEW,
            from: Some(HEALTHY)
        }]
    );
    assert_eq!(passes_at(&run.state, STABLE), None);
}

#[test]
fn a_probation_rebuilt_after_an_observer_restart_waits_for_the_next_run() {
    let restored = Probation {
        open: Some(OpenProbation {
            build: NEW,
            from: Some(HEALTHY),
        }),
        run: None,
        last_healthy: Some(HEALTHY),
    };

    let run = Run::from(restored)
        .at(0, Input::Ready { generation: 1 })
        .at(30, Input::Tick)
        .at(31, ended(1, End::Crashed));
    assert_eq!(run.verdicts, []);

    let run = run
        .at(32, spawned(1, NEW))
        .at(33, Input::Ready { generation: 1 })
        .at(43, Input::Tick);
    assert_eq!(run.verdicts, [Verdict::Passed { build: NEW }]);
}

#[test]
fn the_deadline_is_always_after_now() {
    let inputs = [
        spawned(1, NEW),
        Input::Ready { generation: 1 },
        Input::Tick,
        spawned(2, NEW),
        Input::Ready { generation: 2 },
    ];
    let mut state = Probation {
        last_healthy: Some(HEALTHY),
        ..Probation::default()
    };
    // Minutes far apart, as when the clock jumped while the task slept.
    for (index, input) in inputs.into_iter().enumerate() {
        let now = at(i64::try_from(index).unwrap() * 7);
        state = step(state, input, now, STABLE).0;
        if let Some(deadline) = passes_at(&state, STABLE) {
            assert!(deadline > now, "{input:?} left a deadline at or before now");
        }
    }
}

#[test]
fn there_is_no_going_back_without_a_valid_healthy_build_or_with_the_return_off() {
    assert_eq!(rollback_target(true, Some(HEALTHY), true, Some(NEW)), Ok(HEALTHY));
    assert_eq!(
        rollback_target(false, Some(HEALTHY), true, Some(NEW)),
        Err(Kept::Disabled)
    );
    assert_eq!(
        rollback_target(true, None, true, Some(NEW)),
        Err(Kept::NoHealthyBuild)
    );
    assert_eq!(
        rollback_target(true, Some(HEALTHY), false, Some(NEW)),
        Err(Kept::Broken)
    );
}

#[test]
fn an_automatic_return_never_crosses_the_kvp_build() {
    assert_eq!(
        rollback_target(true, Some(6448), true, Some(6449)),
        Err(Kept::Kvp)
    );
    assert_eq!(rollback_target(true, Some(6448), true, Some(6448)), Ok(6448));
    assert_eq!(rollback_target(true, Some(6449), true, Some(NEW)), Ok(6449));
}
