use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem, Result, Submenu};

use super::actions::Action;
use crate::feature::store::ProcessRecord;

/// Id of the menu's terminal "Exit" item. Distinct from `Action::id()`'s namespace (`Action`
/// only covers per-process actions), so it's a plain constant rather than another `Action`
/// variant.
pub const EXIT_ID: &str = "exit";

/// Builds the tray's full context menu: one submenu per process (mirroring the per-process
/// actions already exposed by the CLI), then a separator and an `Exit` item. `killall` is
/// deliberately not represented here — it's a global action, not a per-process one.
pub fn build(records: &[ProcessRecord]) -> Result<Menu> {
    let menu = Menu::new();
    for record in records {
        menu.append(&process_submenu(record)?)?;
    }
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id(EXIT_ID, "Exit", true, None))?;
    Ok(menu)
}

fn process_submenu(record: &ProcessRecord) -> Result<Submenu> {
    let submenu = Submenu::new(label(record), true);
    submenu.append(&MenuItem::with_id(
        Action::Restart(record.name.clone()).id(),
        "Restart",
        true,
        None,
    ))?;
    submenu.append(&pause_or_resume_item(record))?;
    Ok(submenu)
}

fn pause_or_resume_item(record: &ProcessRecord) -> MenuItem {
    if record.paused {
        MenuItem::with_id(
            Action::Resume(record.name.clone()).id(),
            "Resume",
            true,
            None,
        )
    } else {
        MenuItem::with_id(Action::Pause(record.name.clone()).id(), "Pause", true, None)
    }
}

/// Mirrors `feature::ps::table`'s status cell: the process name, its status, and a `(paused)`
/// suffix when paused.
fn label(record: &ProcessRecord) -> String {
    if record.paused {
        format!("{}  {} (paused)", record.name, record.status.as_str())
    } else {
        format!("{}  {}", record.name, record.status.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{EXIT_ID, build, label};
    use crate::feature::store::{ProcessRecord, ProcessStatus};

    fn record(name: &str, status: ProcessStatus, paused: bool) -> ProcessRecord {
        ProcessRecord {
            status,
            paused,
            ..ProcessRecord::starting(name, "shell", "cargo run")
        }
    }

    #[test]
    fn label_marks_a_paused_process() {
        let paused = record("api", ProcessStatus::Stopped, true);
        let running = record("api", ProcessStatus::Running, false);

        assert_eq!(label(&paused), "api  stopped (paused)");
        assert_eq!(label(&running), "api  running");
    }

    #[test]
    #[ignore = "muda::Menu can only be created on the main thread; cargo test runs on workers"]
    fn build_adds_one_submenu_per_process_plus_a_separator_and_exit() {
        let records = vec![
            record("api", ProcessStatus::Running, false),
            record("worker", ProcessStatus::Stopped, true),
        ];

        let menu = build(&records).expect("menu should build");

        assert_eq!(
            menu.items().len(),
            2 + 1 + 1,
            "2 processes + separator + exit"
        );
        assert_eq!(menu.items()[3].id().0, EXIT_ID);
    }
}
