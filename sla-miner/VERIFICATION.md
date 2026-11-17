# Hand verification: GitHub, last 30 days

Checked on **2026-11-17**, window **2026-10-18T13:54Z → 2026-11-17T13:54Z**, against the
human-readable incident pages on githubstatus.com, which include each incident's update timeline and
GitHub's own postmortem text.

Reproduce: `curl 'localhost:8080/sla?service=GitHub&period=last%2030%20days'` (the miner's default
`basis=component_status`). Add `&basis=incident_window` for the conservative figure.

## Headline

| Basis | uptime_pct | downtime | incidents counted |
|---|---|---|---|
| **component_status** (default) | **99.2053 %** | 343.3 min | 9 |
| incident_window (upper bound) | 97.5822 % | 1044.5 min | 14 |
| v0.1 (created_at→resolved_at, before the fix) | 97.0949 % | 1254 min | 14 |

GitHub's published SLA target is 99.9 % (`compliant: false` on every basis).

**Why the default changed from the incident window:** a Statuspage incident's `impact` is the
*worst* level it ever reached. The incident stays open while backlogs drain, and the components
often spent most of that time in `degraded_performance` rather than an outage. The update log
(`incident_updates[].affected_components`) records each component's `old_status → new_status`, so
we count only time spent in `partial_outage` or `major_outage`. When an incident has no component
data, it falls back to the window (`incidents_window_fallback` reports how many did; it's 0 here).

## Incident-by-incident comparison

Times are UTC. "Postmortem" is the window GitHub states in its final *Resolved* update.

### 1. Oct 21: "Delays for Pull Requests and Webhook deliveries" (critical)
<https://www.githubstatus.com/incidents/nms1dpzmh9d9>

| Source | Window | Minutes |
|---|---|---|
| Status page timeline | 14:52 investigating; Webhooks/PRs `degraded_performance`; 15:15 Actions `degraded_performance`; 16:33 PRs operational; 16:57 Actions + Webhooks operational; 17:09–19:22 audit-log backlog only | n/a |
| Postmortem | "between 13:22 UTC and 19:22 UTC, several GitHub services were **delayed**" | 360 (delays) |
| incident_window | 14:52 → 19:22 (no `monitoring_at`, falls back to `resolved_at`) | 270.0 |
| **component_status** | no component ever left `degraded_performance` | **0** |

**Verdict:** this was a delay incident, not unavailability, and GitHub itself marked every
component as degraded rather than in outage. The old method charged 4.5 h of downtime for it, which
was the largest single source of the overstatement.

### 2. Nov 4: "Incident with Pull Requests, Git Operations and Issues" (major)
<https://www.githubstatus.com/incidents/smtdswd1h2w4>

| Source | Window | Minutes |
|---|---|---|
| Status page timeline | 09:49 investigating; 09:53 "Pull Requests is experiencing **degraded availability**"; 11:30 "API Requests, Git Operations, Issues and Pull Requests … mitigated"; 11:43 monitoring | n/a |
| Postmortem | "between 09:35 UTC and 11:43 UTC, customers experienced intermittent errors and timeouts" | 128 |
| incident_window | 09:49 → 11:43 | 113.9 |
| **component_status** | Pull Requests `partial_outage` 09:53 → 11:30 | **97.1** |

**Verdict:** this matches the timeline exactly. Both methods sit under the postmortem because
GitHub declared the incident 14 min after impact began and the postmortem includes the
Actions tail until 11:43.

### 3. Nov 10: "Incident with Git Operations and Pull Requests" (critical)
<https://www.githubstatus.com/incidents/hnf21m9jkl3c>

| Source | Window | Minutes |
|---|---|---|
| Status page timeline | 16:10 investigating; 16:26 "Actions and Pull Requests are experiencing **degraded availability**"; 17:12 PRs back to degraded performance; 17:28 "Actions is operating normally"; 17:55 resolved | n/a |
| Postmortem | "from approximately 15:00 UTC to 17:55 UTC, GitHub experienced degraded performance" | 175 |
| incident_window | 16:10 → 17:55 (no `monitoring_at`) | 104.2 |
| **component_status** | Actions + PRs `partial_outage` 16:26 → 17:28 (union) | **61.7** |

**Verdict:** this matches the timeline. The postmortem window is longer because it covers
*degraded performance* from 15:00, before the incident was opened.

### 4. Nov 13: "Incident with Pull Requests" (major)
<https://www.githubstatus.com/incidents/1xzmt89vv61l>

| Source | Window | Minutes |
|---|---|---|
| Status page timeline | 15:47 PRs `partial_outage` ("some attempts to merge are timing out"); 16:09 PRs degraded performance; 16:12 "now down to less than 1%"; 17:28 monitoring; 18:29 resolved | n/a |
| Postmortem | not yet published ("A detailed root cause analysis will be shared") | n/a |
| incident_window | 15:47 → 17:28 | 100.8 |
| **component_status** | Pull Requests `partial_outage` 15:47 → 16:09 | **22.1** |

**Verdict:** this matches the timeline. 79 min of the window was a <1 % merge-timeout tail that
GitHub labelled degraded performance.

## Known limitations (also stated in README)

1. **Declared vs actual start.** The status page only knows about an incident once it's opened.
   Postmortems often start 10–90 min earlier (examples 2 and 3), so both bases can *understate*
   downtime that GitHub declared late. We don't parse postmortem prose, because it isn't
   machine-readable and would make the method unreproducible.
2. **"Down" means the provider said so.** We measure provider-admitted outage time, not
   independent probes. This is the evidence an SLA credit claim cites. Partial outages count
   in full, with no weighting by the share of requests affected.
3. **Aggregate service = any component.** `service=GitHub` counts an outage in *any* component,
   including Copilot, which GitHub's SLA doesn't cover. For a claim, ask for the covered
   product, e.g. `service=GitHub Actions` → 99.639 % over the same window (155.9 min).
4. **Coverage.** `incidents.json` returns only the 50 most recent incidents. We page
   `/history.json` and fetch older major/critical incidents individually. Cloudflare's page has
   no `history.json` (HTTP 404), so its answers report `coverage_complete: false` with a note.
   Cloudflare labels nearly all incidents `minor`, so its uptime is usually 100 % by this method.
5. **SLA measurement period.** GitHub's SLA is measured per calendar quarter, while we report the
   requested rolling window. `sla_note` states the provider's measurement period.

## Snapshot of all providers (2026-11-17)

| Service | 30 d uptime | 90 d uptime | SLA target | Published SLA |
|---|---|---|---|---|
| GitHub | 99.2053 | 98.5534 | 99.9 | yes |
| Cloudflare | 100.0 (coverage incomplete) | 100.0 (coverage incomplete) | 100.0 | yes |
| Discord | 99.3664 | 99.2888 | 99.9 | **no** (reference target) |
| Vercel | 100.0 | 99.3687 | 99.99 | yes |
| Twilio | 100.0 | 99.9339 | 99.95 | yes |
| DigitalOcean | 100.0 | 99.7454 | 99.99 | yes |
