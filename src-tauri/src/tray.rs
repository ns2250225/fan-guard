//! 系统托盘：显示 CPU 使用率、CPU 温度与当前异常状态，提供右键菜单

use crate::monitor::{AppState, Snapshot};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

pub const TRAY_ID: &str = "fanguard-tray";

pub struct TrayMenuHandles {
    pub pause: MenuItem<Wry>,
}

/// 生成托盘图标（风扇造型，颜色代表状态）
pub fn make_icon(level: &str) -> Image<'static> {
    let size = 32u32;
    let (r, g, b) = match level {
        "critical" => (239u8, 68u8, 68u8),
        "high" => (249, 115, 22),
        "notice" => (234, 179, 8),
        "paused" => (148, 163, 184),
        _ => (34, 197, 94),
    };
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let c = (size as f64 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 - c;
            let dy = y as f64 - c;
            let d = (dx * dx + dy * dy).sqrt();
            let theta = dy.atan2(dx);
            let idx = ((y * size + x) * 4) as usize;
            let mut alpha = 0.0f64;
            let mut col = (r, g, b);
            if d <= 15.5 && d >= 13.5 {
                // 外圈
                alpha = (1.0 - (d - 14.5).abs()).clamp(0.0, 1.0).max(0.85);
            } else if d < 3.8 {
                // 中心轴
                alpha = 1.0;
                col = (255, 255, 255);
            } else if d < 12.5 {
                // 三片扇叶
                let blade = (3.0 * theta + d * 0.18).sin();
                if blade > 0.25 {
                    alpha = ((blade - 0.25) * 4.0).clamp(0.0, 1.0);
                }
            }
            if alpha > 0.0 {
                rgba[idx] = col.0;
                rgba[idx + 1] = col.1;
                rgba[idx + 2] = col.2;
                rgba[idx + 3] = (alpha * 255.0) as u8;
            }
        }
    }
    Image::new_owned(rgba, size, size)
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        let _ = app.emit_to("main", "resync", ());
    }
}

pub fn navigate(app: &AppHandle, path: &str, action: Option<&str>) {
    show_main(app);
    let _ = app.emit_to(
        "main",
        "navigate",
        serde_json::json!({ "path": path, "action": action }),
    );
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    let state = app.state::<Arc<AppState>>();
    state.paused.store(paused, Ordering::SeqCst);
    state.sensors.set_paused(paused);
    if let Some(h) = app.try_state::<TrayMenuHandles>() {
        let _ = h.pause.set_text(if paused { "恢复监控" } else { "暂停监控" });
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if paused {
            let _ = tray.set_icon(Some(make_icon("paused")));
            let _ = tray.set_tooltip(Some("FanGuard 风扇管家\n监控已暂停"));
            *state.tray_level.lock().unwrap() = "paused".into();
        }
    }
    let _ = app.emit("paused-changed", paused);
    if !paused {
        state.wake_now();
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)?;
    let diagnose = MenuItem::with_id(app, "diagnose", "快速诊断", true, None::<&str>)?;
    let top = MenuItem::with_id(app, "top", "查看高负载进程", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "暂停监控", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出软件", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &diagnose, &top, &sep1, &pause, &settings, &sep2, &quit])?;

    app.manage(TrayMenuHandles { pause: pause.clone() });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(make_icon("normal"))
        .tooltip("FanGuard 风扇管家")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "diagnose" => navigate(app, "/diagnosis", Some("diagnose")),
            "top" => navigate(app, "/processes", Some("high")),
            "pause" => {
                let paused = app.state::<Arc<AppState>>().paused.load(Ordering::SeqCst);
                set_paused(app, !paused);
            }
            "settings" => navigate(app, "/settings", None),
            "quit" => crate::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// 每次采样后更新托盘提示与图标
pub fn update(app: &AppHandle, state: &AppState, s: &Snapshot) {
    let tray = match app.tray_by_id(TRAY_ID) {
        Some(t) => t,
        None => return,
    };
    let temp = s
        .cpu
        .temp
        .map(|t| format!("{:.0}°C", t))
        .unwrap_or_else(|| "暂不支持读取".into());
    let status = match s.anomalies.iter().find(|a| !a.ignored) {
        Some(a) => format!("异常：{}", a.title),
        None => "状态：正常".into(),
    };
    let tip = format!("FanGuard 风扇管家\nCPU：{:.0}%  温度：{}\n{}", s.cpu.usage, temp, status);
    let _ = tray.set_tooltip(Some(tip));

    let mut last = state.tray_level.lock().unwrap();
    if *last != s.level {
        let _ = tray.set_icon(Some(make_icon(&s.level)));
        *last = s.level.clone();
    }
}
