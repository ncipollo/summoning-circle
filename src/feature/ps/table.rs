use chrono::{DateTime, Duration, Utc};

use crate::feature::ps::NO_PROCESSES;
use crate::feature::store::{ProcessRecord, ProcessStatus};

const COLUMNS: [&str; 6] = ["NAME", "STATUS", "PID", "RESTARTS", "UPTIME", "COMMAND"];

/// Renders `records` as a human-readable table, or `NO_PROCESSES` when empty.
/// `now` is a parameter (rather than read internally) so uptime is
/// deterministic to test.
pub fn render(records: &[ProcessRecord], now: DateTime<Utc>) -> String {
    if records.is_empty() {
        return format!("{NO_PROCESSES}\n");
    }

    let rows: Vec<[String; 6]> = records.iter().map(|record| row(record, now)).collect();
    let widths = column_widths(&rows);

    let mut out = format_row(&COLUMNS.map(String::from), &widths);
    for row in &rows {
        out.push_str(&format_row(row, &widths));
    }
    out
}

fn row(record: &ProcessRecord, now: DateTime<Utc>) -> [String; 6] {
    [
        record.name.clone(),
        record.status.as_str().to_string(),
        record
            .pid
            .map_or_else(|| "-".to_string(), |pid| pid.to_string()),
        record.restart_count.to_string(),
        uptime(record, now),
        record.command.clone(),
    ]
}

fn uptime(record: &ProcessRecord, now: DateTime<Utc>) -> String {
    if record.status != ProcessStatus::Running {
        return "-".to_string();
    }
    match record.started_at {
        Some(started_at) => humanize(now - started_at),
        None => "-".to_string(),
    }
}

fn humanize(duration: Duration) -> String {
    let total_minutes = duration.num_minutes().max(0);
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn column_widths(rows: &[[String; 6]]) -> [usize; 6] {
    let mut widths = COLUMNS.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    widths
}

fn format_row(cells: &[String; 6], widths: &[usize; 6]) -> String {
    let mut line = String::new();
    for (cell, width) in cells.iter().zip(widths).take(cells.len() - 1) {
        line.push_str(&format!("{cell:<width$}  "));
    }
    line.push_str(&cells[cells.len() - 1]);
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};

    use super::render;
    use crate::feature::store::{ProcessRecord, ProcessStatus};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }

    fn record(
        status: ProcessStatus,
        pid: Option<u32>,
        started_at: Option<DateTime<Utc>>,
    ) -> ProcessRecord {
        ProcessRecord {
            status,
            pid,
            started_at,
            ..ProcessRecord::starting("api", "shell", "cargo run --release")
        }
    }

    #[test]
    fn renders_placeholder_when_empty() {
        assert!(render(&[], now()).starts_with("No processes tracked."));
    }

    #[test]
    fn renders_running_row_with_uptime() {
        let started_at = now() - chrono::Duration::minutes(72);
        let mut api = record(ProcessStatus::Running, Some(48213), Some(started_at));
        api.restart_count = 2;

        let output = render(&[api], now());

        assert!(output.contains("api"));
        assert!(output.contains("running"));
        assert!(output.contains("48213"));
        assert!(output.contains("1h 12m"));
        assert!(output.contains("cargo run --release"));
    }

    #[test]
    fn renders_dash_for_missing_pid_and_uptime() {
        let mut tunnel = record(ProcessStatus::Exited, None, None);
        tunnel.name = "tunnel".to_string();
        tunnel.restart_count = 7;

        let output = render(&[tunnel], now());
        let data_row = output.lines().nth(1).expect("data row should exist");

        assert!(data_row.contains("tunnel"));
        assert!(data_row.contains("exited"));
        assert!(data_row.contains(" - "));
    }

    #[test]
    fn stale_status_renders_without_uptime() {
        let started_at = now() - chrono::Duration::minutes(30);
        let stale = record(ProcessStatus::Stale, None, Some(started_at));

        let output = render(&[stale], now());

        assert!(output.contains("stale"));
        assert!(!output.contains("30m"));
    }
}
