//! Statuspage.io public API client with history paging. Caching is done at the CDN
//! (Cache-Control: s-maxage=300 on answers), not in process.
//!
//! `/api/v2/incidents.json` only returns the 50 most recent incidents (`?page=` is ignored).
//! When those don't reach back to the window start we page `/history.json?page=N`
//! (about 3 months per page, codes + impact only) and fetch each missing major/critical
//! incident from `/api/v2/incidents/{code}.json` for its timestamps.

use crate::calc::{counts_as_outage, Incident, Ts};
use chrono::{Months, NaiveDate, TimeZone, Utc};
use futures::{stream, StreamExt};
use serde::{de::DeserializeOwned, Deserialize};
use std::collections::HashSet;
use std::time::Instant;

const RECENT_LIMIT: usize = 50;
const MAX_HISTORY_PAGES: u32 = 3;
const MAX_DETAIL_FETCHES: usize = 80;

pub struct Client {
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct IncidentList {
    incidents: Vec<Incident>,
}
#[derive(Deserialize)]
struct IncidentDetail {
    incident: Incident,
}
#[derive(Deserialize)]
struct History {
    #[serde(default)]
    months: Vec<HistoryMonth>,
}
#[derive(Deserialize)]
struct HistoryMonth {
    name: String,
    year: i32,
    #[serde(default)]
    incidents: Vec<HistoryIncident>,
}
#[derive(Deserialize)]
struct HistoryIncident {
    code: String,
    #[serde(default)]
    impact: String,
}
#[derive(Deserialize)]
struct ComponentList {
    components: Vec<ComponentEntry>,
}
#[derive(Deserialize)]
struct ComponentEntry {
    name: String,
    #[serde(default)]
    group: bool,
}

pub struct Fetched {
    pub incidents: Vec<Incident>,
    pub coverage_complete: bool,
    pub coverage_note: Option<String>,
    pub history_pages: u32,
    pub detail_fetches: usize,
}

fn month_start(m: &HistoryMonth) -> Option<Ts> {
    let n: chrono::Month = m.name.parse().ok()?;
    let d = NaiveDate::from_ymd_opt(m.year, n.number_from_month(), 1)?;
    Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?))
}


impl Client {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    async fn get_text(&self, url: &str) -> Result<String, String> {
        let started = Instant::now();
        let res = self.http.get(url).send().await.map_err(|e| format!("fetch {url}: {e}"))?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("fetch {url}: HTTP {status}"));
        }
        let body = res.text().await.map_err(|e| format!("read {url}: {e}"))?;
        tracing::debug!(url, ms = started.elapsed().as_millis() as u64, "upstream fetch");
        Ok(body)
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, String> {
        let body = self.get_text(url).await?;
        serde_json::from_str(&body).map_err(|e| format!("parse {url}: {e}"))
    }

    pub async fn components(&self, base: &str) -> Result<Vec<String>, String> {
        let list: ComponentList = self.get_json(&format!("{base}/api/v2/components.json")).await?;
        Ok(list.components.into_iter().filter(|c| !c.group).map(|c| c.name).collect())
    }

    /// All incidents needed to cover [win_start, now].
    pub async fn incidents_since(&self, base: &str, win_start: Ts) -> Result<Fetched, String> {
        let recent: IncidentList = self.get_json(&format!("{base}/api/v2/incidents.json")).await?;
        let mut incidents = recent.incidents;
        let oldest = incidents.iter().map(|i| i.created_at).min();
        if incidents.len() < RECENT_LIMIT || oldest.is_none_or(|o| o <= win_start) {
            return Ok(Fetched { incidents, coverage_complete: true, coverage_note: None, history_pages: 0, detail_fetches: 0 });
        }

        // Page history for older major/critical incidents.
        let known: HashSet<String> = incidents.iter().map(|i| i.id.clone()).collect();
        let mut codes: Vec<String> = Vec::new();
        let mut reached = false;
        let mut pages = 0;
        let mut note = None;
        for page in 1..=MAX_HISTORY_PAGES {
            let h: History = match self.get_json(&format!("{base}/history.json?page={page}")).await {
                Ok(h) => h,
                Err(e) => {
                    let since = oldest.map(|o| o.date_naive().to_string()).unwrap_or_default();
                    note = Some(format!(
                        "status page API only returns its {RECENT_LIMIT} most recent incidents (back to {since}); \
                         older history unavailable ({e}). Downtime before {since} is not counted."
                    ));
                    break;
                }
            };
            pages = page;
            if h.months.is_empty() {
                reached = true; // no older history exists
                break;
            }
            for m in &h.months {
                let Some(ms) = month_start(m) else { continue };
                let month_end = ms.checked_add_months(Months::new(1)).unwrap_or(ms) + chrono::Duration::days(1);
                if month_end >= win_start {
                    codes.extend(
                        m.incidents
                            .iter()
                            .filter(|i| counts_as_outage(&i.impact) && !known.contains(&i.code))
                            .map(|i| i.code.clone()),
                    );
                }
                if ms <= win_start {
                    reached = true;
                }
            }
            if reached {
                break;
            }
        }
        codes.sort();
        codes.dedup();

        let truncated = codes.len() > MAX_DETAIL_FETCHES;
        codes.truncate(MAX_DETAIL_FETCHES);
        let detail_fetches = codes.len();
        let results: Vec<Result<IncidentDetail, String>> = stream::iter(codes)
            .map(|c| async move { self.get_json::<IncidentDetail>(&format!("{base}/api/v2/incidents/{c}.json")).await })
            .buffer_unordered(8)
            .collect()
            .await;
        let mut failed = 0;
        for r in results {
            match r {
                Ok(d) => incidents.push(d.incident),
                Err(_) => failed += 1,
            }
        }

        let coverage_complete = reached && failed == 0 && !truncated && note.is_none();
        if !coverage_complete && note.is_none() {
            note = Some(match (reached, failed, truncated) {
                (false, _, _) => format!("history before {} not reachable via {} history pages", win_start.date_naive(), MAX_HISTORY_PAGES),
                (_, f, _) if f > 0 => format!("{f} incident detail fetches failed"),
                _ => format!("more than {MAX_DETAIL_FETCHES} older incidents; list truncated"),
            });
        }
        Ok(Fetched { incidents, coverage_complete, coverage_note: note, history_pages: pages, detail_fetches })
    }
}
