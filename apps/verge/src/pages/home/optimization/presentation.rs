use crate::optimization::{Finding, Measurement, Phase, Report, Severity, Snapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Next {
    Check,
    Probe,
    Profiles,
    Network,
    Proxies,
    Compare,
    Review,
    Logs,
    None,
}

pub(super) struct Verdict {
    pub title: &'static str,
    pub detail: &'static str,
    pub next: Next,
    pub severity: Severity,
}

fn verdict(title: &'static str, detail: &'static str, next: Next, severity: Severity) -> Verdict {
    Verdict {
        title,
        detail,
        next,
        severity,
    }
}

pub(super) fn finding<'a>(report: &'a Report, code: &str) -> Option<&'a Finding> {
    report.findings.iter().find(|f| f.code == code)
}

pub(super) fn nodes(report: &Report) -> Vec<(&str, [Option<&Measurement>; 2])> {
    let mut result: Vec<(&str, [Option<&Measurement>; 2])> = Vec::new();
    for m in &report.measurements {
        if m.target >= 2 {
            continue;
        }
        let index = result
            .iter()
            .position(|(name, _)| *name == m.node)
            .unwrap_or_else(|| {
                result.push((&m.node, [None, None]));
                result.len() - 1
            });
        // Later verification replaces earlier attempts in the compact result; details keep all rounds.
        result[index].1[m.target] = Some(m);
    }
    result
}

pub(super) fn summarize(snapshot: &Snapshot) -> Verdict {
    use Next::*;
    use Severity::*;
    match snapshot.phase {
        Phase::Checking => {
            return verdict("health.checking", "health.checking_detail", None, Notice);
        }
        Phase::Optimizing => {
            return verdict("health.comparing", "health.comparing_detail", None, Notice);
        }
        Phase::Verifying => {
            return verdict("health.verifying", "health.verifying_detail", None, Notice);
        }
        Phase::Analyzing => {
            return verdict("health.analyzing", "health.analyzing_detail", None, Notice);
        }
        Phase::Idle => {}
    }
    if snapshot.error.as_deref().is_some_and(|e| {
        matches!(
            e,
            "optimization.apply_unknown" | "optimization.recovery_failed"
        )
    }) || snapshot.report.as_ref().is_some_and(|r| {
        matches!(
            r.outcome.as_deref(),
            Some("optimization.apply_unknown" | "optimization.recovery_failed")
        )
    }) {
        return verdict(
            "health.uncertain",
            "health.uncertain_detail",
            Proxies,
            Warning,
        );
    }
    let Some(r) = &snapshot.report else {
        if snapshot.error.is_some() {
            return verdict(
                "health.incomplete",
                "health.incomplete_detail",
                Check,
                Unknown,
            );
        }
        return verdict("health.ready", "health.ready_detail", Check, Notice);
    };
    if r.cancelled {
        return verdict("health.cancelled", "health.cancelled_detail", Check, Notice);
    }
    if finding(r, "stale").is_some() {
        return verdict("health.stale", "health.stale_detail", Check, Notice);
    }
    if finding(r, "profile").is_some_and(|f| f.severity == Unknown) {
        return verdict(
            "health.no_profile",
            "health.no_profile_detail",
            Profiles,
            Warning,
        );
    }
    if finding(r, "core").is_some_and(|f| f.severity == Warning) {
        return verdict(
            "health.core_down",
            "health.core_down_detail",
            Check,
            Warning,
        );
    }
    if finding(r, "system_proxy").is_some_and(|f| f.severity == Warning) {
        return verdict(
            "health.proxy_mismatch",
            "health.proxy_mismatch_detail",
            Network,
            Warning,
        );
    }
    if finding(r, "tun").is_some_and(|f| f.severity == Warning) {
        return verdict(
            "health.tun_problem",
            "health.tun_problem_detail",
            Network,
            Warning,
        );
    }
    if finding(r, "check_failed").is_some()
        || snapshot
            .error
            .as_deref()
            .is_some_and(|e| e != "optimization.model_failed")
    {
        return verdict(
            "health.incomplete",
            "health.incomplete_detail",
            Check,
            Unknown,
        );
    }
    if r.plan.is_some() {
        return verdict(
            "health.candidate",
            "health.candidate_detail",
            Review,
            Notice,
        );
    }
    if r.outcome.as_deref() == Some("optimization.applied") {
        return verdict("health.switched", "health.switched_detail", None, Pass);
    }
    if matches!(
        r.outcome.as_deref(),
        Some("optimization.restored" | "optimization.not_applied")
    ) {
        return verdict(
            "health.not_fixed",
            "health.not_fixed_detail",
            Compare,
            Warning,
        );
    }
    match r.outcome.as_deref() {
        Some("optimization.no_candidate") => {
            return verdict(
                "health.no_candidate",
                "health.no_candidate_detail",
                Proxies,
                Warning,
            );
        }
        Some("optimization.inconclusive") => {
            return verdict(
                "health.insufficient",
                "health.insufficient_detail",
                Check,
                Notice,
            );
        }
        _ => {}
    }
    if nodes(r).iter().any(|(_, probes)| {
        probes
            .iter()
            .flatten()
            .any(|m| m.delay_ms.is_none_or(|d| d == 0))
    }) {
        return verdict(
            "health.probe_problem",
            "health.probe_problem_detail",
            if r.groups.is_empty() { Check } else { Compare },
            Warning,
        );
    }
    if finding(r, "logs").is_some_and(|f| f.severity == Warning) {
        return verdict(
            "health.log_problem",
            "health.log_problem_detail",
            Logs,
            Warning,
        );
    }
    if finding(r, "direct_mode").is_some() {
        return verdict("health.direct", "health.direct_detail", Network, Notice);
    }
    if finding(r, "probes_disabled").is_some() {
        return verdict(
            "health.local_only",
            "health.local_only_detail",
            Probe,
            Notice,
        );
    }
    let measured = nodes(r);
    if !measured.is_empty()
        && measured.iter().all(|(_, probes)| {
            probes
                .iter()
                .all(|m| m.is_some_and(|m| m.delay_ms.is_some_and(|d| d > 0)))
        })
    {
        return verdict(
            "health.reachable",
            if r.partial {
                "health.reachable_partial"
            } else {
                "health.reachable_detail"
            },
            None,
            Pass,
        );
    }
    verdict(
        "health.incomplete",
        "health.incomplete_detail",
        Check,
        Unknown,
    )
}

pub(super) fn row_text(code: &str, severity: Option<Severity>) -> &'static str {
    match (code, severity) {
        (_, None) => "health.waiting",
        ("profile", Some(Severity::Pass)) => "health.profile_ok",
        ("profile", _) => "health.profile_missing",
        ("core", Some(Severity::Pass)) => "health.core_ok",
        ("core", _) => "health.core_unreachable",
        ("system_proxy", Some(Severity::Pass)) => "health.proxy_ok",
        ("system_proxy", Some(Severity::Notice)) => "health.proxy_unmanaged",
        ("system_proxy", Some(Severity::Warning)) => "health.proxy_different",
        _ => "health.unread",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(findings: &[(&str, Severity)]) -> Snapshot {
        Snapshot {
            report: Some(Report {
                findings: findings
                    .iter()
                    .map(|(code, severity)| Finding {
                        code: (*code).into(),
                        severity: *severity,
                        detail: String::new(),
                    })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn conclusions_prioritize_actionable_faults_and_do_not_invent_connectivity() {
        let local = state(&[
            ("core", Severity::Pass),
            ("probes_disabled", Severity::Unknown),
        ]);
        assert_eq!(summarize(&local).next, Next::Probe);
        assert_ne!(summarize(&local).severity, Severity::Pass);
        for (code, severity, next) in [
            ("profile", Severity::Unknown, Next::Profiles),
            ("core", Severity::Warning, Next::Check),
            ("system_proxy", Severity::Warning, Next::Network),
            ("logs", Severity::Warning, Next::Logs),
        ] {
            assert_eq!(
                summarize(&state(&[
                    (code, severity),
                    ("probes_disabled", Severity::Unknown)
                ]))
                .next,
                next
            );
        }
    }

    #[test]
    fn comparison_outcomes_explain_why_no_switch_was_proposed() {
        for (outcome, title, next) in [
            (
                "optimization.no_candidate",
                "health.no_candidate",
                Next::Proxies,
            ),
            (
                "optimization.inconclusive",
                "health.insufficient",
                Next::Check,
            ),
        ] {
            let mut snapshot = state(&[]);
            let report = snapshot.report.as_mut().unwrap();
            report.outcome = Some(outcome.into());
            report.measurements.push(Measurement {
                node: "failed attempt".into(),
                target: 0,
                delay_ms: None,
                captured_at: 1,
            });
            let verdict = summarize(&snapshot);
            assert_eq!(verdict.title, title);
            assert_eq!(verdict.next, next);
            assert_ne!(verdict.severity, Severity::Pass);
        }
    }

    #[test]
    fn historical_failure_is_not_promoted_to_success_and_model_errors_are_separate() {
        let mut report = Report {
            measurements: (0..2)
                .map(|target| Measurement {
                    node: "a".into(),
                    target,
                    delay_ms: Some(20),
                    captured_at: 1,
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            summarize(&Snapshot {
                report: Some(report.clone()),
                error: Some("optimization.model_failed".into()),
                ..Default::default()
            })
            .title,
            "health.reachable"
        );
        report.findings.push(Finding {
            code: "check_failed".into(),
            severity: Severity::Unknown,
            detail: String::new(),
        });
        assert_eq!(
            summarize(&Snapshot {
                report: Some(report.clone()),
                ..Default::default()
            })
            .title,
            "health.incomplete"
        );
        report.outcome = Some("optimization.apply_unknown".into());
        assert_eq!(
            summarize(&Snapshot {
                report: Some(report),
                ..Default::default()
            })
            .title,
            "health.uncertain"
        );
    }

    #[test]
    fn stable_single_target_failure_is_not_described_as_fluctuation() {
        let report = Report {
            outcome: Some("optimization.inconclusive".into()),
            measurements: [0, 1, 0, 1]
                .into_iter()
                .map(|target| Measurement {
                    node: "a".into(),
                    target,
                    delay_ms: if target == 0 { None } else { Some(20) },
                    captured_at: 1,
                })
                .collect(),
            ..Default::default()
        };
        let verdict = summarize(&Snapshot {
            report: Some(report),
            ..Default::default()
        });
        assert_eq!(verdict.title, "health.insufficient");
        assert_ne!(verdict.severity, Severity::Pass);
    }

    #[test]
    fn latest_results_are_grouped_but_one_target_is_not_a_pass() {
        let mut s = state(&[]);
        let r = s.report.as_mut().unwrap();
        r.measurements.push(Measurement {
            node: "a".into(),
            target: 0,
            delay_ms: None,
            captured_at: 1,
        });
        r.measurements.push(Measurement {
            node: "a".into(),
            target: 0,
            delay_ms: Some(23),
            captured_at: 2,
        });
        assert_eq!(nodes(r).len(), 1);
        assert_eq!(nodes(r)[0].1[0].unwrap().delay_ms, Some(23));
        assert_ne!(summarize(&s).severity, Severity::Pass);
        s.report.as_mut().unwrap().measurements.push(Measurement {
            node: "a".into(),
            target: 1,
            delay_ms: Some(40),
            captured_at: 3,
        });
        assert_eq!(summarize(&s).title, "health.reachable");
        s.report.as_mut().unwrap().findings.push(Finding {
            code: "stale".into(),
            severity: Severity::Unknown,
            detail: String::new(),
        });
        assert_eq!(summarize(&s).next, Next::Check);
    }
}
