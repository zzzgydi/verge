use super::*;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Saved {
    observe: bool,
    model_analysis: bool,
    history: Vec<Report>,
    day: u64,
    runs: u16,
    analyses: u16,
    active_probes: bool,
    sample_cursor: u64,
}

struct State {
    snapshot: Snapshot,
    pending: Option<checks::Pending>,
    invalidated: bool,
    next_check: Instant,
    day: u64,
    runs: u16,
    analyses: u16,
    network: Option<String>,
    last_tick: Instant,
    last_network_check: Instant,
    observed_errors: VecDeque<u64>,
    connections: Option<(u64, usize)>,
    sample_cursor: u64,
    manual_cursor: u64,
}

pub struct Service {
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    pub(crate) epoch: Arc<AtomicU64>,
    directory: PathBuf,
}

impl Service {
    pub fn new(directory: PathBuf) -> Self {
        let saved: Saved =
            crate::config::read_bounded(&directory.join("optimization.json"), 512 * 1024)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
        let mut history = saved.history;
        history.retain(|r| now().saturating_sub(r.captured_at) < 7 * 86400);
        history.truncate(24);
        // Old reports are historical; neither pending changes nor cloud context survive restart.
        for report in &mut history {
            report.plan = None;
            report.explanation = None;
        }
        Self {
            state: Arc::new(Mutex::new(State {
                snapshot: Snapshot {
                    revision: 1,
                    observe: saved.observe,
                    model_analysis: saved.observe && saved.model_analysis,
                    active_probes: saved.active_probes,
                    history,
                    error: directory
                        .join("optimization-intent.json")
                        .exists()
                        .then(|| "optimization.apply_unknown".into()),
                    ..Default::default()
                },
                pending: None,
                invalidated: false,
                next_check: Instant::now() + Duration::from_secs(60),
                day: now() / 86400,
                runs: if saved.day == now() / 86400 {
                    saved.runs
                } else {
                    0
                },
                analyses: if saved.day == now() / 86400 {
                    saved.analyses
                } else {
                    0
                },
                network: None,
                last_tick: Instant::now(),
                last_network_check: Instant::now() - Duration::from_secs(5),
                observed_errors: VecDeque::new(),
                connections: None,
                sample_cursor: saved.sample_cursor,
                manual_cursor: 0,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            epoch: Arc::new(AtomicU64::new(0)),
            directory,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().snapshot.clone()
    }
    pub fn revision(&self) -> u64 {
        self.state.lock().unwrap().snapshot.revision
    }
    pub fn verifying(&self) -> bool {
        self.snapshot().phase == Phase::Verifying
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn shutdown(&self) {
        self.cancel();
        // All controller operations have timeouts. Finish a confirmed transaction before stopping the core.
        while self.verifying() {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.invalidate();
    }

    pub fn observations(&self) -> (Option<usize>, Option<usize>) {
        let state = self.state.lock().unwrap();
        (
            state.snapshot.observe.then(|| {
                state
                    .observed_errors
                    .iter()
                    .filter(|t| now().saturating_sub(**t) <= 300)
                    .count()
            }),
            state
                .connections
                .filter(|(t, _)| now().saturating_sub(*t) <= 10)
                .map(|(_, n)| n),
        )
    }

    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap();
        self.epoch.fetch_add(1, Ordering::Relaxed);
        state.invalidated = true;
        state.pending = None;
        if let Some(report) = &mut state.snapshot.report {
            report.plan = None;
            if !report.findings.iter().any(|f| f.code == "stale") {
                report.findings.push(Finding {
                    code: "stale".into(),
                    severity: Severity::Unknown,
                    detail: String::new(),
                });
            }
        }
        state.next_check = Instant::now() + Duration::from_secs(60);
        state.snapshot.revision += 1;
    }

    pub fn observe(&self, event: &crate::domain::RealtimeEvent) {
        let mut state = self.state.lock().unwrap();
        if !state.snapshot.observe {
            return;
        }
        match event {
            crate::domain::RealtimeEvent::Log(log) if network_failure(log) => {
                while state
                    .observed_errors
                    .front()
                    .is_some_and(|t| now().saturating_sub(*t) > 300)
                {
                    state.observed_errors.pop_front();
                }
                state.observed_errors.push_back(now());
                while state.observed_errors.len() > 100 {
                    state.observed_errors.pop_front();
                }
                if state.observed_errors.len() == 3 {
                    state.next_check = state
                        .next_check
                        .min(Instant::now() + Duration::from_secs(60));
                }
            }
            crate::domain::RealtimeEvent::Connections(sample) => {
                state.connections = Some((now(), sample.connection_count))
            }
            _ => {}
        }
    }

    /// Called at the daemon's existing one-second tick. No network IO on this path.
    pub(crate) fn tick(&mut self, mut context: checks::Context) {
        {
            let mut state = self.state.lock().unwrap();
            let woke = state.last_tick.elapsed() > Duration::from_secs(30);
            state.last_tick = Instant::now();
            if state
                .pending
                .as_ref()
                .is_some_and(|p| p.created.elapsed() > Duration::from_secs(120))
            {
                state.pending = None;
                if let Some(report) = &mut state.snapshot.report {
                    report.plan = None;
                }
                state.snapshot.revision += 1;
            }
            let old_len = state.snapshot.history.len();
            state
                .snapshot
                .history
                .retain(|r| now().saturating_sub(r.captured_at) < 7 * 86400);
            let expired_report = !state.snapshot.busy()
                && state
                    .snapshot
                    .report
                    .as_ref()
                    .is_some_and(|r| now().saturating_sub(r.captured_at) >= 7 * 86400);
            if expired_report {
                state.snapshot.report = None;
                state.pending = None;
            }
            if old_len != state.snapshot.history.len() || expired_report {
                state.snapshot.revision += 1;
                if persist(&self.directory, &saved_state(&state)).is_err() {
                    state.snapshot.error = Some("optimization.storage_failed".into());
                }
            }
            if !state.snapshot.busy()
                && let Some(report) = &mut state.snapshot.report
                && now().saturating_sub(report.captured_at) > 600
                && !report.findings.iter().any(|f| f.code == "stale")
            {
                report.plan = None;
                report.findings.push(Finding {
                    code: "stale".into(),
                    severity: Severity::Unknown,
                    detail: String::new(),
                });
                state.pending = None;
                state.snapshot.revision += 1;
            }
            if woke || state.last_network_check.elapsed() >= Duration::from_secs(5) {
                state.last_network_check = Instant::now();
                let network = checks::fingerprint().ok();
                let changed = woke || (state.network.is_some() && state.network != network);
                if state.network.is_none() || changed {
                    state.next_check = Instant::now() + Duration::from_secs(60);
                }
                state.network = network;
                if changed {
                    self.epoch.fetch_add(1, Ordering::Relaxed);
                    state.pending = None;
                    state.invalidated = true;
                    if let Some(report) = &mut state.snapshot.report {
                        report.plan = None;
                        if !report.findings.iter().any(|f| f.code == "stale") {
                            report.findings.push(Finding {
                                code: "stale".into(),
                                severity: Severity::Unknown,
                                detail: String::new(),
                            });
                        }
                    }
                    state.snapshot.revision += 1;
                }
            }
            while state
                .observed_errors
                .front()
                .is_some_and(|t| now().saturating_sub(*t) > 300)
            {
                state.observed_errors.pop_front();
            }
            if state.day != now() / 86400 {
                state.day = now() / 86400;
                state.runs = 0;
                state.analyses = 0;
            }
            if state.snapshot.busy()
                || !state.snapshot.observe
                || Instant::now() < state.next_check
                || state.runs >= 144
                || state.network.is_none()
                || state.pending.is_some()
            {
                return;
            }
            context.errors = Some(state.observed_errors.len());
            context.connections = state
                .connections
                .filter(|(t, _)| now().saturating_sub(*t) <= 10)
                .map(|(_, n)| n);
            state.runs += 1;
            state.next_check = Instant::now() + Duration::from_secs(600 + now() % 601);
            if persist(&self.directory, &saved_state(&state)).is_err() {
                state.snapshot.error = Some("optimization.storage_failed".into());
                state.snapshot.revision += 1;
                return;
            }
        }
        let _ = self.start_check(context, None, true);
    }

    pub(crate) fn handle(
        &mut self,
        command: Command,
        mut context: checks::Context,
    ) -> Result<Snapshot, AppError> {
        context.active_probes = self.snapshot().active_probes;
        match command {
            Command::AllowProbesAndCheck => {
                self.handle(Command::SetActiveProbes { enabled: true }, context.clone())?;
                self.start_check(context, None, false)?;
            }
            Command::GetState => return Ok(self.snapshot()),
            Command::ClearHistory => {
                let mut state = self.state.lock().unwrap();
                if state.snapshot.busy() {
                    return Err(error("optimization.busy"));
                }
                let mut saved = saved_state(&state);
                saved.history.clear();
                persist(&self.directory, &saved)?;
                clear_intent(&self.directory)?;
                state.snapshot.history.clear();
                state.snapshot.report = None;
                state.snapshot.error = None;
                state.pending = None;
                state.observed_errors.clear();
                state.connections = None;
                state.snapshot.revision += 1;
            }
            Command::Cancel => {
                if self.verifying() {
                    return Err(error("optimization.verifying"));
                }
                self.cancel();
                return Ok(self.snapshot());
            }
            Command::Configure {
                observe,
                model_analysis,
            } => {
                if self.snapshot().busy() {
                    return Err(error("optimization.busy"));
                }
                let mut state = self.state.lock().unwrap();
                let saved = Saved {
                    observe,
                    model_analysis: observe && model_analysis,
                    ..saved_state(&state)
                };
                persist(&self.directory, &saved)?;
                state.snapshot.observe = saved.observe;
                state.snapshot.model_analysis = saved.model_analysis;
                state.snapshot.revision += 1;
                state.next_check = Instant::now() + Duration::from_secs(60);
                state.pending = None;
                if !observe {
                    state.observed_errors.clear();
                    state.connections = None;
                }
                if let Some(report) = &mut state.snapshot.report {
                    report.plan = None;
                }
            }
            Command::SetActiveProbes { enabled } => {
                let mut state = self.state.lock().unwrap();
                if state.snapshot.busy() {
                    return Err(error("optimization.busy"));
                }
                let saved = Saved {
                    active_probes: enabled,
                    ..saved_state(&state)
                };
                persist(&self.directory, &saved)?;
                state.snapshot.active_probes = enabled;
                state.pending = None;
                if let Some(report) = &mut state.snapshot.report {
                    report.plan = None;
                }
                state.next_check = Instant::now() + Duration::from_secs(60);
                state.snapshot.revision += 1;
            }
            Command::Check => self.start_check(context, None, false)?,
            Command::Optimize { group } => {
                if group.len() > 96 {
                    return Err(error("optimization.group_unavailable"));
                }
                self.start_check(context, Some(group), false)?;
            }
            Command::Apply { id } => {
                if !context.active_probes {
                    return Err(error("optimization.probes_required"));
                }
                if self.directory.join("optimization-intent.json").exists() {
                    return Err(error("optimization.apply_unknown"));
                }
                let (pending, mut report) = {
                    let mut state = self.state.lock().unwrap();
                    if state.snapshot.busy() {
                        return Err(error("optimization.busy"));
                    }
                    if state.pending.as_ref().is_none_or(|p| p.plan.id != id) {
                        return Err(error("optimization.expired"));
                    }
                    let pending = state.pending.take().unwrap();
                    let mut report = state
                        .snapshot
                        .report
                        .clone()
                        .ok_or_else(|| error("optimization.expired"))?;
                    report.plan = None;
                    state.snapshot.phase = Phase::Verifying;
                    state.invalidated = false;
                    state.snapshot.error = None;
                    state.snapshot.report = Some(report.clone());
                    state.snapshot.revision += 1;
                    (pending, report)
                };
                let shared = self.state.clone();
                let directory = self.directory.clone();
                let report_id = report.id;
                std::thread::spawn(move || {
                    let mut started = false;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        checks::apply(&context, &pending, &mut report, || {
                            crate::config::restore_runtime_artifact(
                                &directory.join("optimization-intent.json"),
                                &serde_json::to_vec(
                                    &serde_json::json!({"report_id": report_id,"started_at":now()}),
                                )
                                .unwrap(),
                            )
                            .map_err(|_| error("optimization.storage_failed"))?;
                            started = true;
                            Ok(())
                        })
                    }))
                    .unwrap_or_else(|_| Err(error("optimization.apply_unknown")));
                    if result.is_err() && started {
                        report.outcome = Some("optimization.apply_unknown".into());
                    }
                    let confirmed = result.is_ok();
                    report.completed_at = now();
                    finish(
                        &shared,
                        &directory,
                        report,
                        None,
                        result.err(),
                        confirmed,
                        false,
                    );
                });
            }
            Command::Analyze { report_id } => {
                let report = {
                    let mut state = self.state.lock().unwrap();
                    if state.snapshot.busy() {
                        return Err(error("optimization.busy"));
                    }
                    let report = state
                        .snapshot
                        .report
                        .clone()
                        .filter(|r| r.id == report_id)
                        .ok_or_else(|| error("optimization.expired"))?;
                    state.snapshot.phase = Phase::Analyzing;
                    state.snapshot.error = None;
                    state.snapshot.revision += 1;
                    report
                };
                self.cancel = Arc::new(AtomicBool::new(false));
                let shared = self.state.clone();
                let directory = self.directory.clone();
                let cancel = self.cancel.clone();
                std::thread::spawn(move || explain(&shared, &directory, report, &cancel));
            }
        }
        Ok(self.snapshot())
    }

    fn start_check(
        &mut self,
        mut context: checks::Context,
        group: Option<String>,
        background: bool,
    ) -> Result<(), AppError> {
        let id = {
            let mut state = self.state.lock().unwrap();
            if state.snapshot.busy() {
                return Err(error("optimization.busy"));
            }
            context.active_probes = state.snapshot.active_probes;
            if group.is_some() && !context.active_probes {
                return Err(error("optimization.probes_required"));
            }
            if group.is_none() && context.active_probes {
                context.sample_cursor = if background {
                    state.sample_cursor
                } else {
                    state.manual_cursor
                };
                if background {
                    let saved = Saved {
                        sample_cursor: state.sample_cursor.wrapping_add(1),
                        ..saved_state(&state)
                    };
                    persist(&self.directory, &saved)?;
                    state.sample_cursor = saved.sample_cursor;
                } else {
                    state.manual_cursor = state.manual_cursor.wrapping_add(1);
                }
            }
            if let Some(mut old) = state.snapshot.report.take() {
                old.plan = None;
                old.explanation = None;
                state.snapshot.history.insert(0, old);
                state
                    .snapshot
                    .history
                    .retain(|r| now().saturating_sub(r.captured_at) < 7 * 86400);
                state.snapshot.history.truncate(24);
            }
            state.pending = None;
            state.invalidated = false;
            state.snapshot.phase = if group.is_some() {
                Phase::Optimizing
            } else {
                Phase::Checking
            };
            state.snapshot.error = None;
            state.snapshot.revision += 1;
            state.snapshot.revision
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let shared = self.state.clone();
        let directory = self.directory.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                checks::check(
                    &context,
                    id,
                    group.as_deref(),
                    background,
                    &cancel,
                    |report| {
                        let mut state = shared.lock().unwrap();
                        state.snapshot.report = Some(report.clone());
                        state.snapshot.revision += 1;
                    },
                )
            }))
            .unwrap_or_else(|_| Err(error("optimization.failed")));
            match result {
                Ok(result) => {
                    let report = result.report.clone();
                    if finish(
                        &shared,
                        &directory,
                        result.report,
                        result.pending,
                        None,
                        false,
                        background && !cancel.load(Ordering::Relaxed),
                    ) {
                        explain(&shared, &directory, report, &cancel);
                    }
                }
                Err(failure) => {
                    let mut report = shared
                        .lock()
                        .unwrap()
                        .snapshot
                        .report
                        .clone()
                        .unwrap_or_default();
                    report.partial = true;
                    report.completed_at = now();
                    report.cancelled = cancel.load(Ordering::Relaxed);
                    finish(
                        &shared,
                        &directory,
                        report,
                        None,
                        Some(failure),
                        false,
                        false,
                    );
                }
            }
        });
        Ok(())
    }
}

pub(super) fn persisted_history(snapshot: &Snapshot) -> Vec<Report> {
    snapshot
        .report
        .iter()
        .chain(&snapshot.history)
        .filter(|r| now().saturating_sub(r.captured_at) < 7 * 86400)
        .take(24)
        .cloned()
        .map(|mut r| {
            r.plan = None;
            r.explanation = None;
            r.groups.clear();
            // Persist measurements under per-report labels, not subscription names.
            let mut labels = std::collections::BTreeMap::new();
            for m in &mut r.measurements {
                let n = labels.len() + 1;
                m.node = labels
                    .entry(m.node.clone())
                    .or_insert_with(|| format!("node-{n}"))
                    .clone();
            }
            r
        })
        .collect()
}

fn persist(directory: &std::path::Path, saved: &Saved) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(saved).map_err(|_| error("optimization.storage_failed"))?;
    crate::config::restore_runtime_artifact(&directory.join("optimization.json"), &bytes)
        .map_err(|_| error("optimization.storage_failed"))
}

fn saved_state(state: &State) -> Saved {
    Saved {
        observe: state.snapshot.observe,
        model_analysis: state.snapshot.model_analysis,
        history: persisted_history(&state.snapshot),
        day: state.day,
        runs: state.runs,
        analyses: state.analyses,
        active_probes: state.snapshot.active_probes,
        sample_cursor: state.sample_cursor,
    }
}

fn network_failure(log: &crate::domain::LogEvent) -> bool {
    if log.level.eq_ignore_ascii_case("error") {
        return true;
    }
    // Mihomo's logMetadataErr emits warning-level TCP/UDP dial failures.
    // Do not count unrelated configuration warnings or retain the log body.
    (log.level.eq_ignore_ascii_case("warn") || log.level.eq_ignore_ascii_case("warning"))
        && (log.payload.starts_with("[TCP] dial ") || log.payload.starts_with("[UDP] dial "))
        && log.payload.contains(" error: ")
}

fn clear_intent(directory: &std::path::Path) -> Result<(), AppError> {
    match std::fs::remove_file(directory.join("optimization-intent.json")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(error("optimization.storage_failed")),
    }
}

fn finish(
    shared: &Mutex<State>,
    directory: &std::path::Path,
    mut report: Report,
    mut pending: Option<checks::Pending>,
    failure: Option<AppError>,
    confirmed: bool,
    allow_analysis: bool,
) -> bool {
    let mut state = shared.lock().unwrap();
    // Retain terminal failure in history without persisting raw controller errors.
    if failure.is_some() && !report.findings.iter().any(|f| f.code == "check_failed") {
        report.findings.push(Finding {
            code: "check_failed".into(),
            severity: Severity::Unknown,
            detail: String::new(),
        });
    }
    if state.invalidated {
        report.plan = None;
        pending = None;
        if !report.findings.iter().any(|f| f.code == "stale") {
            report.findings.push(Finding {
                code: "stale".into(),
                severity: Severity::Unknown,
                detail: String::new(),
            });
        }
    }
    let analyze = allow_analysis
        && state.snapshot.observe
        && state.snapshot.model_analysis
        && state.analyses < 10
        && !report.cancelled
        && !report.findings.iter().any(|f| f.code == "stale")
        && (report
            .findings
            .iter()
            .any(|f| f.severity == Severity::Warning)
            || report.measurements.iter().any(|m| m.delay_ms.is_none()));
    if analyze {
        state.analyses += 1;
    }
    state.snapshot.report = Some(report);
    state.pending = pending;
    state.snapshot.phase = if analyze {
        Phase::Analyzing
    } else {
        Phase::Idle
    };
    state.snapshot.error = failure.map(|e| e.message);
    state.snapshot.revision += 1;
    let saved = saved_state(&state);
    if persist(directory, &saved).is_err() || (confirmed && clear_intent(directory).is_err()) {
        state.snapshot.error = Some("optimization.storage_failed".into());
        state.snapshot.phase = Phase::Idle;
        return false;
    }
    analyze
}

fn explain(
    shared: &Mutex<State>,
    directory: &std::path::Path,
    mut report: Report,
    cancel: &Arc<AtomicBool>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::ai::explain_health(directory, model_evidence(&report), cancel.clone())
    }))
    .unwrap_or_else(|_| Err(error("optimization.model_failed")));
    match result {
        Ok(text) => report.explanation = Some(text),
        Err(_) => {
            let mut state = shared.lock().unwrap();
            state.snapshot.phase = Phase::Idle;
            state.snapshot.error = Some("optimization.model_failed".into());
            state.snapshot.revision += 1;
            return;
        }
    }
    let mut state = shared.lock().unwrap();
    // Preserve a separately confirmed plan; analysis has no write capability.
    if let Some(current) = &mut state.snapshot.report
        && current.id == report.id
    {
        current.explanation = report.explanation;
    }
    state.snapshot.phase = Phase::Idle;
    state.snapshot.revision += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_failure_survives_archiving_restart_and_redaction() {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let directory =
            std::env::temp_dir().join(format!("verge-health-failure-{}", hex::encode(random)));
        let context = checks::Context {
            socket: directory.join("missing.sock"),
            profiles: crate::config::FileProfileStore::open(directory.clone()).unwrap(),
            services: Vec::new(),
            recovery: directory.join("recovery.json"),
            target: None,
            tun_connected: false,
            errors: None,
            connections: None,
            epoch: Arc::new(AtomicU64::new(0)),
            active_probes: false,
            sample_cursor: 0,
        };
        let mut service = Service::new(directory.clone());
        let report = Report {
            id: 1,
            captured_at: now(),
            partial: true,
            measurements: (0..2)
                .map(|target| Measurement {
                    node: "private-node".into(),
                    target,
                    delay_ms: Some(20),
                    captured_at: now(),
                })
                .collect(),
            ..Default::default()
        };
        finish(
            &service.state,
            &directory,
            report,
            None,
            Some(error("private controller read error")),
            false,
            false,
        );
        let failed = |r: &Report| r.findings.iter().any(|f| f.code == "check_failed");
        assert!(failed(service.snapshot().report.as_ref().unwrap()));
        assert!(failed(
            &Service::new(directory.clone()).snapshot().history[0]
        ));
        service.start_check(context.clone(), None, false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while service.snapshot().busy() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(failed(&service.snapshot().history[0]));
        let saved = std::fs::read_to_string(directory.join("optimization.json")).unwrap();
        assert!(saved.contains("check_failed"));
        assert!(!saved.contains("private"));
        service.handle(Command::ClearHistory, context).unwrap();
        assert!(service.snapshot().report.is_none());
        assert!(service.snapshot().history.is_empty());
        assert!(
            Service::new(directory.clone())
                .snapshot()
                .history
                .is_empty()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn background_cursor_survives_restart_and_manual_checks_do_not_advance_it() {
        let mut random = [0; 8];
        getrandom::fill(&mut random).unwrap();
        let directory =
            std::env::temp_dir().join(format!("verge-rotation-{}", hex::encode(random)));
        let context = checks::Context {
            socket: directory.join("missing.sock"),
            profiles: crate::config::FileProfileStore::open(directory.clone()).unwrap(),
            services: Vec::new(),
            recovery: directory.join("recovery.json"),
            target: None,
            tun_connected: false,
            errors: None,
            connections: None,
            epoch: Arc::new(AtomicU64::new(0)),
            active_probes: false,
            sample_cursor: 0,
        };
        let mut service = Service::new(directory.clone());
        service
            .handle(Command::SetActiveProbes { enabled: true }, context.clone())
            .unwrap();
        for (background, expected) in [(true, 1), (false, 1), (true, 2)] {
            service
                .start_check(context.clone(), None, background)
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while service.snapshot().busy() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(service.state.lock().unwrap().sample_cursor, expected);
            service = Service::new(directory.clone());
            assert_eq!(service.state.lock().unwrap().sample_cursor, expected);
        }
        service.handle(Command::ClearHistory, context).unwrap();
        assert_eq!(
            Service::new(directory.clone())
                .state
                .lock()
                .unwrap()
                .sample_cursor,
            2
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn network_warning_burst_advances_check_and_expires_old_errors() {
        let directory = std::env::temp_dir().join(format!("verge-observer-{}", std::process::id()));
        let service = Service::new(directory);
        {
            let mut state = service.state.lock().unwrap();
            state.snapshot.observe = true;
            state.next_check = Instant::now() + Duration::from_secs(900);
            state.observed_errors.extend([now() - 301; 3]);
        }
        for _ in 0..3 {
            service.observe(&crate::domain::RealtimeEvent::Log(
                crate::domain::LogEvent {
                    level: "warning".into(),
                    payload: "[TCP] dial Proxy error: timeout".into(),
                },
            ));
        }
        assert_eq!(service.observations().0, Some(3));
        assert!(
            service.state.lock().unwrap().next_check <= Instant::now() + Duration::from_secs(60)
        );
    }
}
