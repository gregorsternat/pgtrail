//! Portable incident reports retain observations, notes and conservative comparisons.
use crate::{compare, diagnostics, model::Snapshot, report, store::Incident};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TimelineId {
    Capture(i64),
    Note(DateTime<Utc>, usize),
}

pub(crate) struct TimelineEntry {
    pub(crate) id: TimelineId,
    pub(crate) at: DateTime<Utc>,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) capture_id: Option<i64>,
}

/// Full chronology shared by navigation and rendering; no captured payload I/O.
pub(crate) fn timeline(incident: &Incident) -> Vec<TimelineEntry> {
    let mut entries: Vec<_> = incident
        .captures
        .iter()
        .map(|capture| TimelineEntry {
            id: TimelineId::Capture(capture.id),
            at: capture.captured_at,
            title: format!("Capture #{}: {}", capture.id, capture.label),
            detail: format!(
                "Capture #{}: {}\nObserved: {}\nSource: {}\nCollection: {}\n{}\nEnter: inspect this capture offline. Esc returns to this chronology.",
                capture.id,
                capture.label,
                capture.captured_at.to_rfc3339(),
                capture.source,
                if capture.complete { "complete (not a health verdict)" } else { "partial; inspect coverage in the capture" },
                incident.capture_notes.get(&capture.id).map(|note| format!("Annotation: {note}")).unwrap_or_default(),
            ),
            capture_id: Some(capture.id),
        })
        .collect();
    let mut occurrences = BTreeMap::new();
    for note in &incident.notes {
        let occurrence = occurrences.entry(note.created_at).or_insert(0_usize);
        entries.push(TimelineEntry {
            id: TimelineId::Note(note.created_at, *occurrence),
            at: note.created_at,
            title: format!("Note: {}", note.text),
            detail: format!(
                "Operator note · {}\n\n{}",
                note.created_at.to_rfc3339(),
                note.text
            ),
            capture_id: None,
        });
        *occurrence += 1;
    }
    // Stable sorting retains insertion order for notes with equal timestamps.
    entries.sort_by_key(|entry| (entry.at, entry.capture_id.is_none()));
    entries
}

pub(crate) fn markdown(incident: &Incident, captures: &[(i64, Snapshot)]) -> String {
    let summary = &incident.summary;
    let mut output = format!(
        "# Incident {}: {}\n\n- Created: {}\n- Status: {}\n- Captures: {}\n- Notes: {}\n\n",
        summary.id,
        escape(&summary.title),
        summary.created_at.to_rfc3339(),
        summary
            .closed_at
            .map(|at| format!("Closed at {}", at.to_rfc3339()))
            .unwrap_or_else(|| "Open".into()),
        summary.capture_count,
        summary.note_count
    );
    output.push_str("This report contains local observations and operator notes. Observed symptoms are evidence for investigation, not proof of root cause. Capture times describe completed collection intervals; notes use the time they were added locally.\n\n");

    let mut ordered: Vec<_> = captures
        .iter()
        .filter(|(id, _)| incident.captures.iter().any(|capture| capture.id == *id))
        .collect();
    ordered.sort_by_key(|(id, snapshot)| (snapshot.completed_at, *id));
    output
        .push_str("## Timeline\n\n| Time (UTC) | Event | Evidence / note |\n| --- | --- | --- |\n");
    let mut events = Vec::new();
    let mut previous: Option<&Snapshot> = None;
    for (id, snapshot) in &ordered {
        let label = incident
            .captures
            .iter()
            .find(|capture| capture.id == *id)
            .map(|capture| capture.label.as_str())
            .unwrap_or_default();
        let mut evidence = match snapshot.activity.available() {
            Some(sessions) => {
                let active = sessions
                    .iter()
                    .filter(|session| session.state.as_deref() == Some("active"))
                    .count();
                let blocked = sessions
                    .iter()
                    .filter(|session| !session.blockers.is_empty())
                    .count();
                let oldest = sessions
                    .iter()
                    .filter_map(|session| session.transaction_age_ms)
                    .max()
                    .map(|age| format!("{age} ms"))
                    .unwrap_or_else(|| "unavailable / NULL".into());
                format!(
                    "{} sessions; {active} active; {blocked} blocked; oldest observed transaction {oldest}",
                    sessions.len()
                )
            }
            None => "Session metrics unavailable".into(),
        };
        if !snapshot.is_complete() {
            evidence.push_str("; partial capture");
        }
        let analysis = diagnostics::analyze(snapshot, previous);
        if analysis.findings.is_empty() {
            evidence.push_str("; no findings from available metrics (not a health verdict)");
        } else {
            evidence.push_str(&format!(
                "; {} findings: {}",
                analysis.findings.len(),
                analysis
                    .findings
                    .iter()
                    .take(3)
                    .map(|finding| finding.title.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if snapshot.source.system_identifier.is_none() {
            evidence.push_str("; server identity falls back to endpoint/database");
        }
        if previous.is_some_and(|before| {
            before.source.server_started_at != snapshot.source.server_started_at
        }) {
            evidence.push_str(
                "; SERVER RESTART observed, interval counters across this boundary are unavailable",
            );
        }
        events.push((
            snapshot.completed_at,
            0,
            format!(
                "| {} | [Capture #{id}: {}](#capture-{id}) | {} |\n",
                snapshot.completed_at.to_rfc3339(),
                escape(label),
                escape(&evidence)
            ),
        ));
        previous = Some(snapshot);
    }
    for note in &incident.notes {
        events.push((
            note.created_at,
            1,
            format!(
                "| {} | Operator note | {} |\n",
                note.created_at.to_rfc3339(),
                escape(&note.text)
            ),
        ));
    }
    events.sort_by_key(|(timestamp, kind, _)| (*timestamp, *kind));
    for (_, _, row) in events {
        output.push_str(&row);
    }
    if incident.captures.is_empty() && incident.notes.is_empty() {
        output.push_str("| — | No observations yet | Attach a capture or add a note to begin the investigation. |\n");
    }
    output.push('\n');

    if ordered.len() > 1 {
        output.push_str("## First-to-last comparison\n\n");
        if let (Some((before_id, before)), Some((after_id, after))) =
            (ordered.first(), ordered.last())
        {
            output.push_str(&format!("Compare [capture #{before_id}](#capture-{before_id}) with [capture #{after_id}](#capture-{after_id}).\n\n"));
            output.push_str(&nested(&report::comparison_markdown(&compare::compare(
                before, after,
            ))));
        }
    } else {
        output.push_str(
            "## Comparison\n\nAt least two captures are needed for an interval comparison.\n\n",
        );
    }

    output.push_str("## Captured evidence\n\n");
    for capture in &incident.captures {
        output.push_str(&format!(
            "<a id=\"capture-{}\"></a>\n\n### Capture #{}: {}\n\n",
            capture.id,
            capture.id,
            escape(&capture.label)
        ));
        if let Some(note) = incident.capture_notes.get(&capture.id) {
            output.push_str(&format!("Capture annotation: {}\n\n", escape(note)));
        }
        if let Some((_, snapshot)) = ordered.iter().find(|(id, _)| *id == capture.id) {
            output.push_str(&nested(&report::snapshot_markdown(snapshot)));
        } else {
            output.push_str("Captured payload unavailable in this export. No diagnostic values have been inferred.\n\n");
        }
    }
    output
}

fn nested(report: &str) -> String {
    report
        .lines()
        .map(|line| {
            if line.starts_with('#') {
                format!("##{line}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect::<String>()
        + "\n"
}

fn escape(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\\' | '|' | '*' | '_' | '`' | '[' | ']' | '#' => {
                output.push('\\');
                output.push(character);
            }
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' => {
                output.push(' ')
            }
            c if c.is_control() => output.push(' '),
            c => output.push(c),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compare::tests::snapshot,
        store::{IncidentNote, IncidentSummary, SnapshotSummary},
    };
    use chrono::Duration;

    #[test]
    fn timeline_contains_old_events_full_notes_and_stable_capture_identity() {
        let before = snapshot();
        let mut incident = Incident {
            summary: IncidentSummary {
                id: 1,
                title: "Incident".into(),
                created_at: before.completed_at,
                closed_at: None,
                capture_count: 7,
                note_count: 8,
            },
            notes: (0..8)
                .map(|i| IncidentNote {
                    created_at: before.completed_at + Duration::seconds(i),
                    text: format!("Note {i}: {}END", "long note ".repeat(200)),
                })
                .collect(),
            captures: (1..=7)
                .rev()
                .map(|id| SnapshotSummary {
                    id,
                    label: format!("capture {id}"),
                    captured_at: before.completed_at + Duration::seconds(id),
                    source: "local".into(),
                    complete: true,
                })
                .collect(),
            capture_notes: [(1, "Original context".into())].into_iter().collect(),
        };
        let entries = timeline(&incident);
        assert_eq!(entries.len(), 15);
        assert!(entries.windows(2).all(|pair| pair[0].at <= pair[1].at));
        assert!(entries[0].detail.ends_with("END"));
        let capture = entries
            .iter()
            .find(|entry| entry.capture_id == Some(1))
            .unwrap();
        assert_eq!(capture.id, TimelineId::Capture(1));
        assert!(capture.detail.contains("Original context"));
        let note_id = entries[0].id.clone();
        incident.captures.reverse();
        incident.notes.push(IncidentNote {
            created_at: before.completed_at,
            text: "Equal timestamp".into(),
        });
        assert_eq!(timeline(&incident)[0].id, note_id);
        assert_ne!(timeline(&incident)[1].id, note_id);
    }

    #[test]
    fn report_orders_evidence_links_comparison_and_explains_restart() {
        let before = snapshot();
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        after.source.server_started_at += Duration::seconds(1);
        let captures = vec![(2, after.clone()), (1, before.clone())];
        let incident = Incident {
            summary: IncidentSummary {
                id: 7,
                title: "checkout <incident>".into(),
                created_at: before.started_at,
                closed_at: None,
                capture_count: 2,
                note_count: 1,
            },
            notes: vec![IncidentNote {
                created_at: before.completed_at + Duration::seconds(1),
                text: "Changed [pool](https://example.com)\n# injected\u{1b}".into(),
            }],
            captures: captures
                .iter()
                .map(|(id, snapshot)| SnapshotSummary {
                    id: *id,
                    label: format!("capture {id}"),
                    captured_at: snapshot.completed_at,
                    source: "local".into(),
                    complete: true,
                })
                .collect(),
            capture_notes: [(1, "Baseline before the fix <review>".into())]
                .into_iter()
                .collect(),
        };
        let markdown = markdown(&incident, &captures);
        assert!(markdown.contains("checkout &lt;incident&gt;"));
        assert!(markdown.contains("SERVER RESTART"));
        assert!(markdown.contains("Source identity differs"));
        assert!(markdown.contains("(#capture-1)"));
        assert!(markdown.contains("Capture annotation: Baseline before the fix &lt;review&gt;"));
        assert!(markdown.find("[Capture #1").unwrap() < markdown.find("Operator note").unwrap());
        assert!(markdown.find("Operator note").unwrap() < markdown.find("[Capture #2").unwrap());
        assert!(!markdown.contains('\u{1b}'));
        assert!(!markdown.contains("\n# injected"));
    }
}
