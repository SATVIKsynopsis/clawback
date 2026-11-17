//! Pure uptime math over Statuspage incidents. No I/O.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub type Ts = DateTime<Utc>;

#[derive(Clone, Debug, Deserialize)]
pub struct Component {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AffectedComponent {
    pub name: String,
    pub old_status: Option<String>,
    pub new_status: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IncidentUpdate {
    pub created_at: Ts,
    #[serde(default)]
    pub affected_components: Option<Vec<AffectedComponent>>,
}

/// Subset of a Statuspage v2 incident.
#[derive(Clone, Debug, Deserialize)]
pub struct Incident {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub impact: String,
    pub created_at: Ts,
    pub started_at: Option<Ts>,
    pub monitoring_at: Option<Ts>,
    pub resolved_at: Option<Ts>,
    #[serde(default)]
    pub components: Vec<Component>,
    #[serde(default)]
    pub incident_updates: Vec<IncidentUpdate>,
}

/// How downtime is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Time any (matching) component spent in partial_outage/major_outage, per the incident's
    /// update log. Falls back to `IncidentWindow` for incidents without component data.
    ComponentStatus,
    /// Whole incident window (started_at -> monitoring_at). Conservative upper bound.
    IncidentWindow,
}

impl Basis {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().replace('-', "_").as_str() {
            "component_status" | "component" | "status" => Some(Self::ComponentStatus),
            "incident_window" | "incident" | "window" => Some(Self::IncidentWindow),
            _ => None,
        }
    }
}

pub fn is_outage_status(s: &str) -> bool {
    matches!(s, "partial_outage" | "major_outage")
}

/// Per-component outage spans reconstructed from `incident_updates[].affected_components`.
/// Returns `None` if the incident carries no component status data at all.
/// A component is "down" from the update that moves it into partial/major outage until the
/// update that moves it out; anything still down at the incident end closes at
/// monitoring_at / resolved_at / now (same end rule as `outage_span`).
pub fn component_spans(i: &Incident, now: Ts) -> Option<Vec<(Ts, Ts, String)>> {
    let mut updates: Vec<&IncidentUpdate> = i.incident_updates.iter().collect();
    if !updates.iter().any(|u| u.affected_components.as_ref().is_some_and(|c| !c.is_empty())) {
        return None;
    }
    updates.sort_by_key(|u| u.created_at);
    let incident_end = i.monitoring_at.or(i.resolved_at).unwrap_or(now).min(now);
    let mut open: Vec<(String, Ts)> = Vec::new();
    let mut spans = Vec::new();
    for u in updates {
        for c in u.affected_components.iter().flatten() {
            let was = c.old_status.as_deref().is_some_and(is_outage_status);
            let now_down = c.new_status.as_deref().is_some_and(is_outage_status);
            let pos = open.iter().position(|(n, _)| n == &c.name);
            match (pos, now_down) {
                (None, true) => {
                    // already down before this update (e.g. first update reports old_status=outage)
                    let start = if was { i.started_at.unwrap_or(i.created_at).min(u.created_at) } else { u.created_at };
                    open.push((c.name.clone(), start));
                }
                (Some(k), false) => {
                    let (name, start) = open.remove(k);
                    if u.created_at > start {
                        spans.push((start, u.created_at, name));
                    }
                }
                _ => {}
            }
        }
    }
    for (name, start) in open {
        let end = incident_end.max(start);
        if end > start {
            spans.push((start, end, name));
        }
    }
    Some(spans)
}

/// Only provider-declared major/critical incidents count as downtime.
/// minor/none are "degraded performance" and are excluded.
pub fn counts_as_outage(impact: &str) -> bool {
    matches!(impact, "major" | "critical")
}

/// Case-insensitive substring match on any affected component name.
pub fn matches_component(i: &Incident, component: &str) -> bool {
    let c = component.to_lowercase();
    i.components.iter().any(|x| x.name.to_lowercase().contains(&c))
}

/// Outage span for one incident:
///   start = started_at (fallback created_at, also used if started_at is not before the end)
///   end   = monitoring_at (fix deployed), fallback resolved_at, fallback `now` (ongoing)
pub fn outage_span(i: &Incident, now: Ts) -> Option<(Ts, Ts)> {
    let mut end = i.monitoring_at.or(i.resolved_at).unwrap_or(now);
    let start = match i.started_at {
        Some(s) if s < end => s,
        _ => i.created_at,
    };
    if end <= start {
        // monitoring_at recorded before the start: fall back to resolved_at
        end = i.resolved_at.filter(|r| *r > start).unwrap_or(end);
    }
    (end > start).then_some((start, end.min(now)))
}

pub fn clip(span: (Ts, Ts), win_start: Ts, win_end: Ts) -> Option<(Ts, Ts)> {
    let s = span.0.max(win_start);
    let e = span.1.min(win_end);
    (e > s).then_some((s, e))
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Outage {
    pub start: Ts,
    pub end: Ts,
    pub minutes: f64,
    pub incidents: Vec<String>,
}

/// Merge overlapping/touching spans so concurrent incidents are not double-counted.
pub fn merge(mut spans: Vec<(Ts, Ts, String)>) -> Vec<Outage> {
    spans.sort_by_key(|s| (s.0, s.1));
    let mut out: Vec<Outage> = Vec::new();
    for (s, e, name) in spans {
        match out.last_mut() {
            Some(last) if s <= last.end => {
                if e > last.end {
                    last.end = e;
                }
                if !last.incidents.contains(&name) {
                    last.incidents.push(name);
                }
            }
            _ => out.push(Outage { start: s, end: e, minutes: 0.0, incidents: vec![name] }),
        }
    }
    for o in &mut out {
        o.minutes = round((o.end - o.start).num_seconds() as f64 / 60.0, 1);
    }
    out
}

pub fn uptime_pct(down: Duration, window: Duration) -> f64 {
    let total = window.num_seconds() as f64;
    if total <= 0.0 {
        return 100.0;
    }
    let down = (down.num_seconds() as f64).clamp(0.0, total);
    round((1.0 - down / total) * 100.0, 4)
}

fn round(x: f64, dp: i32) -> f64 {
    let f = 10f64.powi(dp);
    (x * f).round() / f
}

#[derive(Debug)]
pub struct Computation {
    pub outages: Vec<Outage>,
    pub downtime: Duration,
    pub incidents_counted: usize,
    /// incidents measured by their whole window because they had no component status data
    pub incidents_window_fallback: usize,
    pub uptime_pct: f64,
}

pub fn compute(incidents: &[Incident], win_start: Ts, win_end: Ts, component: Option<&str>, basis: Basis) -> Computation {
    let comp_lc = component.map(str::to_lowercase);
    let mut spans: Vec<(Ts, Ts, String)> = Vec::new();
    let mut counted = 0;
    let mut fallback = 0;
    for i in incidents.iter().filter(|i| counts_as_outage(&i.impact)) {
        let per_component = match basis {
            Basis::ComponentStatus => component_spans(i, win_end),
            Basis::IncidentWindow => None,
        };
        let mine: Vec<(Ts, Ts, String)> = match per_component {
            Some(cs) => cs
                .into_iter()
                .filter(|(_, _, name)| comp_lc.as_ref().is_none_or(|c| name.to_lowercase().contains(c)))
                .filter_map(|(s, e, name)| clip((s, e), win_start, win_end).map(|(s, e)| (s, e, format!("{} [{}]", i.name, name))))
                .collect(),
            None => {
                if component.is_some_and(|c| !matches_component(i, c)) {
                    continue;
                }
                let Some((s, e)) = outage_span(i, win_end).and_then(|sp| clip(sp, win_start, win_end)) else { continue };
                if basis == Basis::ComponentStatus {
                    fallback += 1;
                }
                vec![(s, e, i.name.clone())]
            }
        };
        if !mine.is_empty() {
            counted += 1;
            spans.extend(mine);
        }
    }
    let outages = merge(spans);
    let downtime = outages.iter().fold(Duration::zero(), |acc, o| acc + (o.end - o.start));
    Computation {
        uptime_pct: uptime_pct(downtime, win_end - win_start),
        outages,
        downtime,
        incidents_counted: counted,
        incidents_window_fallback: fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Ts {
        s.parse().unwrap()
    }

    fn inc(impact: &str, created: &str, started: Option<&str>, monitoring: Option<&str>, resolved: Option<&str>, comps: &[&str]) -> Incident {
        Incident {
            id: created.into(),
            name: format!("{impact}@{created}"),
            impact: impact.into(),
            created_at: t(created),
            started_at: started.map(t),
            monitoring_at: monitoring.map(t),
            resolved_at: resolved.map(t),
            components: comps.iter().map(|n| Component { name: n.to_string() }).collect(),
            incident_updates: vec![],
        }
    }

    const NOW: &str = "2026-11-17T00:00:00Z";

    #[test]
    fn span_uses_started_to_monitoring() {
        let i = inc("major", "2026-11-13T15:47:12Z", Some("2026-11-13T15:47:00Z"), Some("2026-11-13T17:28:00Z"), Some("2026-11-13T18:29:00Z"), &[]);
        assert_eq!(outage_span(&i, t(NOW)), Some((t("2026-11-13T15:47:00Z"), t("2026-11-13T17:28:00Z"))));
    }

    #[test]
    fn span_falls_back_to_resolved_then_now() {
        let i = inc("critical", "2026-11-10T16:10:00Z", None, None, Some("2026-11-10T17:55:00Z"), &[]);
        assert_eq!(outage_span(&i, t(NOW)), Some((t("2026-11-10T16:10:00Z"), t("2026-11-10T17:55:00Z"))));
        let ongoing = inc("critical", "2026-11-16T23:00:00Z", None, None, None, &[]);
        assert_eq!(outage_span(&ongoing, t(NOW)), Some((t("2026-11-16T23:00:00Z"), t(NOW))));
    }

    #[test]
    fn span_ignores_bogus_started_at() {
        // Real GitHub incident 2026-10-07: started_at was set equal to resolved_at, after monitoring_at.
        let i = inc("critical", "2026-10-07T15:14:45Z", Some("2026-10-07T16:25:14Z"), Some("2026-10-07T15:49:57Z"), Some("2026-10-07T16:25:14Z"), &[]);
        assert_eq!(outage_span(&i, t(NOW)), Some((t("2026-10-07T15:14:45Z"), t("2026-10-07T15:49:57Z"))));
    }

    #[test]
    fn clip_to_window() {
        let w = (t("2026-11-01T00:00:00Z"), t("2026-11-02T00:00:00Z"));
        assert_eq!(clip((t("2026-10-31T23:00:00Z"), t("2026-11-01T01:00:00Z")), w.0, w.1), Some((w.0, t("2026-11-01T01:00:00Z"))));
        assert_eq!(clip((t("2026-10-30T00:00:00Z"), t("2026-10-31T00:00:00Z")), w.0, w.1), None);
        assert_eq!(clip((t("2026-11-01T23:00:00Z"), t("2026-11-03T00:00:00Z")), w.0, w.1), Some((t("2026-11-01T23:00:00Z"), w.1)));
    }

    #[test]
    fn merge_overlaps_and_keeps_gaps() {
        let m = merge(vec![
            (t("2026-11-01T10:00:00Z"), t("2026-11-01T11:00:00Z"), "a".into()),
            (t("2026-11-01T10:30:00Z"), t("2026-11-01T12:00:00Z"), "b".into()),
            (t("2026-11-01T11:15:00Z"), t("2026-11-01T11:45:00Z"), "c".into()), // fully inside
            (t("2026-11-01T14:00:00Z"), t("2026-11-01T14:30:00Z"), "d".into()),
        ]);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].start, m[0].end, m[0].minutes), (t("2026-11-01T10:00:00Z"), t("2026-11-01T12:00:00Z"), 120.0));
        assert_eq!(m[0].incidents, vec!["a", "b", "c"]);
        assert_eq!(m[1].minutes, 30.0);
    }

    #[test]
    fn uptime_math() {
        assert_eq!(uptime_pct(Duration::zero(), Duration::days(30)), 100.0);
        // 43.2 minutes in 30 days = exactly 99.9%
        assert_eq!(uptime_pct(Duration::seconds(2592), Duration::days(30)), 99.9);
        assert_eq!(uptime_pct(Duration::days(1), Duration::days(10)), 90.0);
        assert_eq!(uptime_pct(Duration::days(20), Duration::days(10)), 0.0);
    }

    #[test]
    fn compute_filters_impact_component_and_window() {
        let ws = t("2026-11-01T00:00:00Z");
        let we = t(NOW);
        let incs = vec![
            inc("minor", "2026-11-05T00:00:00Z", None, None, Some("2026-11-05T05:00:00Z"), &["Actions"]),
            inc("major", "2026-11-06T00:00:00Z", None, Some("2026-11-06T01:00:00Z"), Some("2026-11-06T02:00:00Z"), &["Actions"]),
            inc("critical", "2026-11-06T00:30:00Z", None, None, Some("2026-11-06T01:30:00Z"), &["Pages"]),
            inc("major", "2026-10-31T23:00:00Z", None, None, Some("2026-11-01T00:30:00Z"), &["Git Operations"]),
        ];
        let all = compute(&incs, ws, we, None, Basis::IncidentWindow);
        assert_eq!(all.incidents_counted, 3);
        assert_eq!(all.downtime, Duration::minutes(90 + 30));
        assert_eq!(all.outages.len(), 2);

        let actions = compute(&incs, ws, we, Some("actions"), Basis::IncidentWindow);
        assert_eq!(actions.incidents_counted, 1);
        assert_eq!(actions.downtime, Duration::minutes(60));
    }

    fn upd(at: &str, changes: &[(&str, &str, &str)]) -> IncidentUpdate {
        IncidentUpdate {
            created_at: t(at),
            affected_components: Some(
                changes
                    .iter()
                    .map(|(n, o, nw)| AffectedComponent { name: n.to_string(), old_status: Some(o.to_string()), new_status: Some(nw.to_string()) })
                    .collect(),
            ),
        }
    }

    #[test]
    fn component_status_counts_only_outage_states() {
        // Mirrors GitHub 2026-11-13 "Incident with Pull Requests": partial_outage 15:47-16:09, then degraded.
        let mut i = inc("major", "2026-11-13T15:47:00Z", Some("2026-11-13T15:47:00Z"), Some("2026-11-13T17:28:00Z"), Some("2026-11-13T18:29:00Z"), &["Pull Requests"]);
        i.incident_updates = vec![
            upd("2026-11-13T15:47:00Z", &[("Pull Requests", "operational", "partial_outage")]),
            upd("2026-11-13T15:49:00Z", &[("Pull Requests", "partial_outage", "partial_outage")]),
            upd("2026-11-13T16:09:00Z", &[("Pull Requests", "partial_outage", "degraded_performance")]),
            upd("2026-11-13T17:28:00Z", &[("Pull Requests", "degraded_performance", "operational")]),
        ];
        let spans = component_spans(&i, t(NOW)).unwrap();
        assert_eq!(spans, vec![(t("2026-11-13T15:47:00Z"), t("2026-11-13T16:09:00Z"), "Pull Requests".to_string())]);
        let ws = t("2026-11-01T00:00:00Z");
        let c = compute(&[i.clone()], ws, t(NOW), None, Basis::ComponentStatus);
        assert_eq!(c.downtime, Duration::minutes(22));
        let w = compute(&[i], ws, t(NOW), None, Basis::IncidentWindow);
        assert_eq!(w.downtime, Duration::minutes(101));
    }

    #[test]
    fn degraded_only_incident_is_zero_downtime() {
        // Mirrors GitHub 2026-10-21: labelled critical, but components only ever degraded_performance.
        let mut i = inc("critical", "2026-10-21T14:52:00Z", None, None, Some("2026-10-21T19:22:00Z"), &["Actions"]);
        i.incident_updates = vec![
            upd("2026-10-21T14:52:00Z", &[("Actions", "operational", "degraded_performance")]),
            upd("2026-10-21T16:57:00Z", &[("Actions", "degraded_performance", "operational")]),
        ];
        let c = compute(&[i], t("2026-10-01T00:00:00Z"), t(NOW), None, Basis::ComponentStatus);
        assert_eq!(c.downtime, Duration::zero());
        assert_eq!(c.incidents_counted, 0);
    }

    #[test]
    fn component_filter_and_open_until_incident_end() {
        let mut i = inc("critical", "2026-11-09T18:12:00Z", None, Some("2026-11-09T18:43:00Z"), Some("2026-11-09T18:59:00Z"), &["Actions", "Pages"]);
        i.incident_updates = vec![upd("2026-11-09T18:12:00Z", &[("Actions", "operational", "major_outage"), ("Pages", "operational", "degraded_performance")])];
        let ws = t("2026-11-01T00:00:00Z");
        // never closed in the log -> closes at monitoring_at
        assert_eq!(compute(&[i.clone()], ws, t(NOW), Some("actions"), Basis::ComponentStatus).downtime, Duration::minutes(31));
        assert_eq!(compute(&[i.clone()], ws, t(NOW), Some("pages"), Basis::ComponentStatus).downtime, Duration::zero());
        // no component data -> window fallback, reported
        i.incident_updates.clear();
        let c = compute(&[i], ws, t(NOW), None, Basis::ComponentStatus);
        assert_eq!((c.downtime, c.incidents_window_fallback), (Duration::minutes(31), 1));
    }
}
