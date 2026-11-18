//! FanGuard 风扇管家 —— Rust 核心层

mod commands;
mod db;
mod diagnosis;
mod hardware;
mod hwinfo;
mod hwinfo_manager;
mod monitor;
mod pawnio;
mod pdh;
mod process;
mod process_ops;
mod sensors;
mod settings;
mod tray;
mod util;

use monitor::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use tauri::{AppHandle, Manager, WindowEvent};

/// 退出软件：停止监控线程与传感器 Sidecar
pub fn quit(app: &AppHandle) {
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        state.quitting.store(true, Ordering::SeqCst);
        state.wake_now();
        state.hwinfo_mgr.on_exit();
        state.sensors.shutdown();
    }
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let start_minimized = args.iter().any(|a| a == "--minimized");
    let elevated_relaunch = args.iter().any(|a| a == "--elevated");

    // 以管理员身份重启时，旧实例在 UAC 通过后立即退出：稍等片刻再注册单实例，避免与旧实例冲突
    if elevated_relaunch {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        tray::show_main(app);
    }));

    builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("FanGuard"));
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| config_dir.clone());
            let settings_path = settings::settings_path(&config_dir);
            let settings = settings::Settings::load(&settings_path);

            // 用户选择了「始终以管理员身份运行」：请求 UAC 后由新的提权实例接管；用户取消则继续以普通权限运行
            if settings.run_as_admin && !elevated_relaunch && !process_ops::is_elevated() {
                let mut args = vec!["--elevated".to_string()];
                if start_minimized {
                    args.push("--minimized".into());
                }
                if process_ops::relaunch_as_admin_with(&args.join(" ")).is_ok() {
                    std::process::exit(0);
                }
            }

            // 同步开机启动状态
            {
                use tauri_plugin_autostart::ManagerExt;
                let al = app.autolaunch();
                let enabled = al.is_enabled().unwrap_or(false);
                if settings.autostart && !enabled {
                    let _ = al.enable();
                } else if !settings.autostart && enabled {
                    let _ = al.disable();
                }
            }

            let db = match db::Db::open(&data_dir.join("fanguard.db")) {
                Ok(d) => {
                    d.cleanup(settings.history_retention_days);
                    Some(d)
                }
                Err(e) => {
                    eprintln!("数据库打开失败: {}", e);
                    None
                }
            };

            let static_info = hardware::load_static_info();
            let sensors = sensors::SensorHub::new(settings.sensor_interval_ms);
            sensors.set_source(&settings.sensor_source);
            let hwinfo_mgr = hwinfo_manager::HwinfoManager::new(sensors.clone(), settings.hwinfo_auto);
            let auto_monitor = settings.auto_start_monitoring;
            let state = Arc::new(AppState {
                settings: RwLock::new(settings),
                settings_path,
                static_info,
                db: Mutex::new(db),
                engine: Mutex::new(diagnosis::DiagEngine::new()),
                latest: RwLock::new(None),
                engines: Mutex::new(Default::default()),
                sensors: sensors.clone(),
                hwinfo_mgr: hwinfo_mgr.clone(),
                paused: AtomicBool::new(!auto_monitor),
                wake: (Mutex::new(false), Condvar::new()),
                quitting: AtomicBool::new(false),
                tray_level: Mutex::new(String::new()),
            });
            app.manage(state);
            sensors.start();
            hwinfo_mgr.start();
            if !auto_monitor {
                sensors.set_paused(true);
            }

            tray::create(app.handle())?;
            if !auto_monitor {
                tray::set_paused(app.handle(), true);
            }
            monitor::start_monitor(app.handle().clone());

            if let Some(w) = app.get_webview_window("main") {
                if !start_minimized {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let to_tray = app
                    .try_state::<Arc<AppState>>()
                    .map(|s| s.settings().minimize_to_tray)
                    .unwrap_or(true);
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    quit(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_static_info,
            commands::get_snapshot,
            commands::get_realtime,
            commands::get_settings,
            commands::save_settings,
            commands::set_whitelist,
            commands::set_paused,
            commands::get_paused,
            commands::refresh_now,
            commands::get_process_detail,
            commands::get_process_threads,
            commands::get_signature,
            commands::get_file_info,
            commands::open_file_location,
            commands::show_file_properties,
            commands::preview_kill,
            commands::kill_process,
            commands::run_diagnosis,
            commands::ignore_anomalies,
            commands::get_history,
            commands::export_csv,
            commands::clear_history,
            commands::get_sensor_raw,
            commands::get_sensor_status,
            commands::is_elevated,
            commands::relaunch_admin,
            commands::quit_app,
            commands::hide_to_tray,
            commands::close_window,
            commands::hwinfo_launch,
            commands::hwinfo_import,
            commands::hwinfo_open_download,
            commands::pawnio_status,
            commands::pawnio_install,
            commands::pawnio_open_site,
        ])
        .build(tauri::generate_context!())
        .expect("FanGuard 启动失败")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<Arc<AppState>>() {
                    state.quitting.store(true, Ordering::SeqCst);
                    state.hwinfo_mgr.on_exit();
        state.sensors.shutdown();
                }
            }
        });
}
