mod actions;
mod menu;
mod poll;

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use objc2_core_foundation::CFRunLoop;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::MenuEvent;
use tray_icon::{TrayIcon, TrayIconBuilder};

use actions::Action;
use poll::Poller;

/// How often the menu is rebuilt from the store. The issue calls for "every 1-2s".
const POLL_INTERVAL: Duration = Duration::from_secs(2);

enum UserEvent {
    Menu(MenuEvent),
}

/// Drives the menu-bar icon until the user quits it. `tao::EventLoop::run` never returns on
/// macOS (it tears the process down itself once `ControlFlow::Exit` is observed), so the only
/// way this function returns normally is if setup fails before `run` is reached.
pub fn run(db_path: &Path, on_exit: impl FnOnce() + 'static) -> Result<()> {
    let poller = Poller::new(db_path);
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        // The tray lives for the process's whole lifetime, so the loop is never gone.
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let mut tray_icon: Option<TrayIcon> = None;
    let mut on_exit = Some(on_exit);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL);

        match event {
            Event::NewEvents(StartCause::Init) => init_tray_icon(&poller, &mut tray_icon),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                refresh_tray_icon(&poller, tray_icon.as_ref())
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                handle_menu_event(&event, control_flow, &mut on_exit);
            }
            _ => {}
        }
    })
}

/// Built once the event loop is actually running, per
/// <https://github.com/tauri-apps/tray-icon/issues/90>. The explicit `CFRunLoop` wake-up below
/// is required for the icon to actually appear on macOS (`tao` only exposes a redraw method on
/// `Window`, which a tray icon doesn't have).
fn init_tray_icon(poller: &Poller, tray_icon: &mut Option<TrayIcon>) {
    let records = poller.tick().unwrap_or_default();
    let built = TrayIconBuilder::new()
        .with_menu(Box::new(
            menu::build(&records).expect("initial tray menu should build"),
        ))
        .with_title("circle")
        .with_tooltip("summoning-circle")
        .build()
        .expect("tray icon should build");
    *tray_icon = Some(built);

    if let Some(main) = CFRunLoop::main() {
        main.wake_up();
    }
}

fn refresh_tray_icon(poller: &Poller, tray_icon: Option<&TrayIcon>) {
    let Some(tray_icon) = tray_icon else { return };
    let records = match poller.tick() {
        Ok(records) => records,
        Err(error) => {
            tracing::warn!(%error, "could not poll tracked processes for the tray menu");
            return;
        }
    };
    match menu::build(&records) {
        Ok(menu) => tray_icon.set_menu(Some(Box::new(menu))),
        Err(error) => tracing::warn!(%error, "could not rebuild the tray menu"),
    }
}

fn handle_menu_event(
    event: &MenuEvent,
    control_flow: &mut ControlFlow,
    on_exit: &mut Option<impl FnOnce()>,
) {
    let id = event.id.0.as_str();
    if id == menu::EXIT_ID {
        *control_flow = ControlFlow::Exit;
        if let Some(on_exit) = on_exit.take() {
            on_exit();
        }
        return;
    }
    let Some(action) = Action::parse(id) else {
        return;
    };
    if let Err(error) = actions::dispatch(&action) {
        tracing::warn!(%error, ?action, "tray action failed");
    }
}
