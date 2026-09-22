//! Portable incident reports retain observations, notes and conservative comparisons.
use crate::{compare, diagnostics, model::Snapshot, report, store::Incident};

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
