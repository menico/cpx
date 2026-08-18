//! The menu-bar app. Presentation only: every decision comes from cpx-core.

mod commands;
mod view;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, WindowEvent};

/// A popover normally hides when it loses focus, but a native folder picker
/// takes focus too — hiding then would dismiss the window mid-task.
static PICKER_OPEN: AtomicBool = AtomicBool::new(false);

/// Open a folder picker and return the chosen path.
///
/// This lives in Rust rather than the frontend so the app stays frontmost
/// while the sheet is up, and so hide-on-blur can be suspended around it.
#[tauri::command]
async fn pick_directory(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;

    PICKER_OPEN.store(true, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_title("Choose a directory to bind")
        .pick_folder(move |path| {
            let _ = tx.send(path.map(|p| p.to_string()));
        });
    let chosen = rx.recv().unwrap_or(None);
    PICKER_OPEN.store(false, Ordering::SeqCst);
    chosen
}

#[tauri::command]
fn hide_window(window: tauri::Window) {
    let _ = window.hide();
}

/// Replace the stub `PATH` macOS gives a Finder-launched app with the one a
/// terminal would have.
///
/// Without this the app cannot find the Claude binary, reports direnv and
/// `~/.local/bin` as missing, and — worst — writes wrappers that exec a bare
/// `claude`, losing the absolute path that stops a wrapper directory from
/// shadowing the real binary.
fn restore_user_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    let Some(login) = cpx_core::env_path::login_path() else {
        return;
    };
    std::env::set_var("PATH", cpx_core::env_path::merge_paths(&login, &current));
}

/// Print what the app resolved from its environment, then exit.
///
/// A GUI app's environment is invisible from a terminal, so when the health
/// checks disagree with reality this is how to see what it actually had.
fn diagnose() {
    println!("PATH      {}", std::env::var("PATH").unwrap_or_default());
    match cpx_core::install::layout_from_env() {
        Ok(layout) => {
            println!("home      {}", layout.home.display());
            println!("root      {}", layout.root.display());
            match cpx_core::discovery::resolve_claude_binary(
                &std::env::var("PATH").unwrap_or_default(),
                &layout,
            ) {
                Some(path) => println!("claude    {}", path.display()),
                None => println!("claude    NOT FOUND"),
            }
        }
        Err(e) => println!("layout    {e}"),
    }
    match cpx_core::install::which("direnv") {
        Some(path) => println!("direnv    {}", path.display()),
        None => println!("direnv    NOT FOUND"),
    }
}

pub fn run() {
    restore_user_path();

    if std::env::args().any(|arg| arg == "--diagnose") {
        diagnose();
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::is_initialised,
            commands::initialise,
            commands::profiles,
            commands::profile,
            commands::plan,
            commands::apply,
            commands::bindings,
            commands::bind,
            commands::unbind,
            commands::checks,
            commands::add_profile,
            commands::remove_profile,
            commands::clone_profile,
            commands::set_field,
            commands::set_resource,
            commands::adoption_candidates,
            commands::adopt,
            commands::config_path,
            commands::auth,
            commands::reveal,
            pick_directory,
            hide_window,
        ])
        .setup(setup)
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(false) = event {
                if !PICKER_OPEN.load(Ordering::SeqCst) && std::env::var("CPX_DEV_SHOW").is_err() {
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("the app should build")
        .run(|_app, event| {
            // Closing the window leaves the app in the menu bar rather than
            // quitting: that is what the tray icon is for.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let open = MenuItem::with_id(app, "open", "Open cpx", true, None::<&str>)?;
    let config = MenuItem::with_id(app, "config", "Edit config.toml…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit cpx", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &config, &quit])?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    // Shown on launch only when asked for: handy while developing, since the
    // tray icon cannot be clicked from a script.
    if std::env::var("CPX_DEV_SHOW").is_ok() {
        let handle = app.handle().clone();
        show_window(&handle);
        // Placement is applied once the window server has actually realised
        // the window; during `setup` it is silently dropped.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.set_always_on_top(true);
                let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition {
                    x: 40.0,
                    y: 60.0,
                }));
            }
        });
    }

    TrayIconBuilder::with_id("cpx")
        .icon(icon)
        // A template image is tinted by macOS, so it stays legible in both
        // light and dark menu bars.
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "config" => {
                if let Ok(path) = commands::config_path() {
                    use tauri_plugin_opener::OpenerExt;
                    let _ = app.opener().open_path(path, None::<&str>);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                let app = tray.app_handle();
                let Some(window) = app.get_webview_window("main") else {
                    return;
                };
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                    return;
                }
                let _ = window.show();
                position_under_tray(&window, rect);
                let _ = window.set_focus();
            }
        })
        .build(app)?;

    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Centre the window horizontally under the tray icon, just below the menu bar.
///
/// Must be called after `show`: macOS discards a position set while the window
/// is hidden, which would leave the popover centred on screen.
fn position_under_tray(window: &tauri::WebviewWindow, rect: tauri::Rect) {
    use tauri::{PhysicalPosition, Position};

    let Ok(size) = window.outer_size() else { return };
    let (tray_x, tray_bottom) = match (rect.position, rect.size) {
        (Position::Physical(p), tauri::Size::Physical(s)) => {
            (p.x as f64 + s.width as f64 / 2.0, p.y as f64 + s.height as f64)
        }
        (Position::Logical(p), tauri::Size::Logical(s)) => {
            let scale = window.scale_factor().unwrap_or(1.0);
            (
                (p.x + s.width / 2.0) * scale,
                (p.y + s.height) * scale,
            )
        }
        _ => return,
    };

    let x = (tray_x - size.width as f64 / 2.0).max(8.0);
    let _ = window.set_position(Position::Physical(PhysicalPosition {
        x: x as i32,
        y: (tray_bottom + 6.0) as i32,
    }));
}
