//! 进程操作：结束进程 / 强制结束 / 结束进程树 / 详情 / 线程 / 文件信息 / 签名

use crate::process::{
    classify, is_sensitive, open_process, query_image_path, query_processes, ProcKind, RawProcess, RawThread,
};
use crate::util::{from_wide, wide, CREATE_NO_WINDOW};
use serde::Serialize;
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use windows_sys::Win32::Foundation::{GetLastError, BOOL, ERROR_ACCESS_DENIED, HWND, LPARAM, WAIT_OBJECT_0};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcessToken, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, ShellExecuteW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, GW_OWNER, SW_SHOWNORMAL,
    WM_CLOSE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillMode {
    /// 正常结束：向进程窗口发送关闭消息
    Graceful,
    /// 强制结束：TerminateProcess
    Force,
    /// 结束进程树：递归强制结束子进程及自身
    Tree,
}

impl KillMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "graceful" => Some(Self::Graceful),
            "force" => Some(Self::Force),
            "tree" => Some(Self::Tree),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Graceful => "结束进程",
            Self::Force => "强制结束进程",
            Self::Tree => "结束进程树",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Force => "force",
            Self::Tree => "tree",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillFailure {
    pub pid: u32,
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillResult {
    pub success: bool,
    pub message: String,
    /// 是否因权限不足失败（提示以管理员身份运行）
    pub requires_admin: bool,
    /// 正常结束失败时是否建议强制结束
    pub can_force: bool,
    pub killed: Vec<u32>,
    pub failed: Vec<KillFailure>,
    pub process_name: String,
}

impl KillResult {
    fn fail(name: &str, msg: impl Into<String>) -> Self {
        Self {
            success: false,
            message: msg.into(),
            requires_admin: false,
            can_force: false,
            killed: vec![],
            failed: vec![],
            process_name: name.to_string(),
        }
    }
}

/// 校验 PID 对应的进程身份（PID + 创建时间），避免 PID 重用误杀
pub fn verify_identity(pid: u32, create_time: i64) -> Result<RawProcess, String> {
    let list = query_processes(false, Some(pid));
    match list.into_iter().next() {
        Some(p) if p.create_time == create_time => Ok(p),
        Some(_) => Err("该 PID 已被其他进程重用，原进程已经退出，操作已取消".into()),
        None => Err("进程已经退出".into()),
    }
}

pub fn protection_check(p: &RawProcess, path: Option<&str>) -> Result<(), String> {
    if p.pid == std::process::id() {
        return Err("不能结束 FanGuard 自身".into());
    }
    if classify(p.pid, &p.name, p.session_id, path) == ProcKind::Critical {
        return Err(format!(
            "{} 是关键 Windows 系统进程，结束它可能导致系统崩溃或蓝屏，FanGuard 已阻止该操作",
            p.name
        ));
    }
    Ok(())
}

struct EnumCtx {
    pid: u32,
    hwnds: Vec<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam as *mut EnumCtx);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == ctx.pid && IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
        ctx.hwnds.push(hwnd);
    }
    1
}

fn top_windows(pid: u32) -> Vec<HWND> {
    let mut ctx = EnumCtx { pid, hwnds: vec![] };
    unsafe {
        EnumWindows(Some(enum_proc), &mut ctx as *mut EnumCtx as LPARAM);
    }
    ctx.hwnds
}

fn wait_exit(pid: u32, ms: u32) -> bool {
    match open_process(pid, PROCESS_SYNCHRONIZE) {
        Some(h) => unsafe { WaitForSingleObject(h.0, ms) == WAIT_OBJECT_0 },
        // 无法打开句柄：再次查询确认是否仍然存在
        None => query_processes(false, Some(pid)).is_empty(),
    }
}

pub(crate) fn terminate(pid: u32) -> Result<(), (String, bool)> {
    let h = match open_process(pid, PROCESS_TERMINATE | PROCESS_SYNCHRONIZE) {
        Some(h) => h,
        None => {
            let err = unsafe { GetLastError() };
            if err == ERROR_ACCESS_DENIED {
                return Err(("拒绝访问：需要管理员权限才能结束该进程".into(), true));
            }
            return Err((format!("无法打开进程（错误码 {}）", err), false));
        }
    };
    let ok = unsafe { TerminateProcess(h.0, 1) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if err == ERROR_ACCESS_DENIED {
            return Err(("拒绝访问：该进程受系统保护或需要管理员权限".into(), true));
        }
        return Err((format!("TerminateProcess 失败（错误码 {}）", err), false));
    }
    unsafe { WaitForSingleObject(h.0, 3000) };
    Ok(())
}

/// 递归收集子进程（按深度优先，最深的子进程在前）
pub fn collect_descendants(all: &[RawProcess], root_pid: u32, root_create: i64) -> Vec<RawProcess> {
    let mut by_parent: HashMap<u32, Vec<&RawProcess>> = HashMap::new();
    for p in all {
        by_parent.entry(p.ppid).or_default().push(p);
    }
    let mut out = Vec::new();
    fn walk<'a>(
        map: &HashMap<u32, Vec<&'a RawProcess>>,
        pid: u32,
        create: i64,
        out: &mut Vec<RawProcess>,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        if let Some(children) = map.get(&pid) {
            for c in children {
                // 子进程必须晚于父进程创建，否则说明父 PID 已被重用
                if c.pid != pid && c.create_time >= create {
                    walk(map, c.pid, c.create_time, out, depth + 1);
                    out.push((*c).clone());
                }
            }
        }
    }
    walk(&by_parent, root_pid, root_create, &mut out, 0);
    out
}

pub fn kill(pid: u32, create_time: i64, mode: KillMode) -> KillResult {
    let target = match verify_identity(pid, create_time) {
        Ok(p) => p,
        Err(e) => return KillResult::fail("", e),
    };
    let path = query_image_path(pid);
    if let Err(e) = protection_check(&target, path.as_deref()) {
        return KillResult::fail(&target.name, e);
    }
    let name = target.name.clone();

    match mode {
        KillMode::Graceful => {
            let wins = top_windows(pid);
            if wins.is_empty() {
                let mut r = KillResult::fail(
                    &name,
                    "该进程没有可接收关闭请求的窗口（后台进程），无法正常结束。可选择「强制结束进程」。",
                );
                r.can_force = true;
                return r;
            }
            for w in &wins {
                unsafe { PostMessageW(*w, WM_CLOSE, 0, 0) };
            }
            if wait_exit(pid, 5000) {
                KillResult {
                    success: true,
                    message: format!("{} (PID {}) 已正常退出", name, pid),
                    requires_admin: false,
                    can_force: false,
                    killed: vec![pid],
                    failed: vec![],
                    process_name: name,
                }
            } else {
                let mut r = KillResult::fail(
                    &name,
                    "已发送关闭请求，但进程在 5 秒内未退出（可能正在等待保存或无响应）。可选择「强制结束进程」。",
                );
                r.can_force = true;
                r
            }
        }
        KillMode::Force => match terminate(pid) {
            Ok(_) => KillResult {
                success: true,
                message: format!("{} (PID {}) 已被强制结束", name, pid),
                requires_admin: false,
                can_force: false,
                killed: vec![pid],
                failed: vec![],
                process_name: name,
            },
            Err((msg, admin)) => {
                let mut r = KillResult::fail(&name, msg);
                r.requires_admin = admin;
                r
            }
        },
        KillMode::Tree => {
            let all = query_processes(false, None);
            let descendants = collect_descendants(&all, pid, target.create_time);
            let mut killed = vec![];
            let mut failed = vec![];
            let mut need_admin = false;
            for d in descendants.iter().chain(std::iter::once(&target)) {
                let dpath = query_image_path(d.pid);
                if let Err(e) = protection_check(d, dpath.as_deref()) {
                    failed.push(KillFailure {
                        pid: d.pid,
                        name: d.name.clone(),
                        reason: e,
                    });
                    continue;
                }
                match terminate(d.pid) {
                    Ok(_) => killed.push(d.pid),
                    Err((msg, admin)) => {
                        need_admin |= admin;
                        failed.push(KillFailure {
                            pid: d.pid,
                            name: d.name.clone(),
                            reason: msg,
                        });
                    }
                }
            }
            let success = killed.contains(&pid);
            let message = if failed.is_empty() {
                format!("已结束进程树：{} 及 {} 个子进程", name, killed.len().saturating_sub(1))
            } else {
                format!(
                    "已结束 {} 个进程，{} 个进程未能结束",
                    killed.len(),
                    failed.len()
                )
            };
            KillResult {
                success,
                message,
                requires_admin: need_admin,
                can_force: false,
                killed,
                failed,
                process_name: name,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 结束前预检查（供确认对话框展示）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillPreview {
    pub pid: u32,
    pub name: String,
    pub allowed: bool,
    pub blocked_reason: Option<String>,
    /// 警告级别：none / service / sensitive
    pub warning: String,
    pub warning_text: Option<String>,
    pub has_windows: bool,
    pub children: Vec<ChildBrief>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildBrief {
    pub pid: u32,
    pub name: String,
    pub protected: bool,
}

pub fn preview(pid: u32, create_time: i64) -> Result<KillPreview, String> {
    let target = verify_identity(pid, create_time)?;
    let path = query_image_path(pid);
    let blocked = protection_check(&target, path.as_deref()).err();
    let kind = classify(pid, &target.name, target.session_id, path.as_deref());
    let (warning, warning_text) = if kind == ProcKind::Service {
        (
            "service",
            Some("这是一个系统服务/后台服务进程（Session 0），结束后相关系统功能可能失效，部分服务会被系统自动重启。".to_string()),
        )
    } else if kind == ProcKind::System || is_sensitive(&target.name) {
        (
            "sensitive",
            Some(format!(
                "{} 是 Windows 系统组件，结束后可能导致桌面、任务栏、输入法或音频暂时异常。",
                target.name
            )),
        )
    } else {
        ("none", None)
    };
    let all = query_processes(false, None);
    let children = collect_descendants(&all, pid, target.create_time)
        .into_iter()
        .map(|c| {
            let cp = query_image_path(c.pid);
            ChildBrief {
                protected: classify(c.pid, &c.name, c.session_id, cp.as_deref()) == ProcKind::Critical,
                pid: c.pid,
                name: c.name,
            }
        })
        .collect();
    Ok(KillPreview {
        pid,
        name: target.name.clone(),
        allowed: blocked.is_none(),
        blocked_reason: blocked,
        warning: warning.into(),
        warning_text,
        has_windows: !top_windows(pid).is_empty(),
        children,
    })
}

// ---------------------------------------------------------------------------
// 线程
// ---------------------------------------------------------------------------

pub fn threads(pid: u32, create_time: i64) -> Result<Vec<RawThread>, String> {
    let list = query_processes(true, Some(pid));
    let p = list.into_iter().next().ok_or("进程已经退出")?;
    if p.create_time != create_time {
        return Err("进程已经退出（PID 已被重用）".into());
    }
    let mut t = p.thread_list;
    t.sort_by(|a, b| b.cpu_time_ms.partial_cmp(&a.cpu_time_ms).unwrap_or(std::cmp::Ordering::Equal));
    Ok(t)
}

// ---------------------------------------------------------------------------
// 文件信息
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub size: Option<u64>,
    pub modified: Option<i64>,
    pub created: Option<i64>,
    pub description: Option<String>,
    pub company: Option<String>,
    pub product: Option<String>,
    pub version: Option<String>,
    pub copyright: Option<String>,
    pub original_name: Option<String>,
}

fn ver_string(data: &[u8], lang: &str, key: &str) -> Option<String> {
    let sub = wide(&format!("\\StringFileInfo\\{}\\{}", lang, key));
    let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut len: u32 = 0;
    let ok = unsafe { VerQueryValueW(data.as_ptr() as *const _, sub.as_ptr(), &mut ptr, &mut len) };
    if ok == 0 || ptr.is_null() || len == 0 {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u16, len as usize) };
    let s = from_wide(slice).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn file_info(path: &str) -> FileInfo {
    let mut info = FileInfo::default();
    if let Ok(meta) = std::fs::metadata(path) {
        info.size = Some(meta.len());
        let to_ms = |t: std::io::Result<std::time::SystemTime>| {
            t.ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
        };
        info.modified = to_ms(meta.modified());
        info.created = to_ms(meta.created());
    }
    let wpath = wide(path);
    let mut handle = 0u32;
    let size = unsafe { GetFileVersionInfoSizeW(wpath.as_ptr(), &mut handle) };
    if size == 0 {
        return info;
    }
    let mut data = vec![0u8; size as usize];
    let ok = unsafe { GetFileVersionInfoW(wpath.as_ptr(), 0, size, data.as_mut_ptr() as *mut _) };
    if ok == 0 {
        return info;
    }
    // 读取语言代码页
    let mut langs: Vec<String> = vec![];
    let sub = wide("\\VarFileInfo\\Translation");
    let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut len: u32 = 0;
    if unsafe { VerQueryValueW(data.as_ptr() as *const _, sub.as_ptr(), &mut ptr, &mut len) } != 0 && !ptr.is_null() {
        let n = len as usize / 4;
        let pairs = unsafe { std::slice::from_raw_parts(ptr as *const u16, n * 2) };
        for i in 0..n {
            langs.push(format!("{:04x}{:04x}", pairs[i * 2], pairs[i * 2 + 1]));
        }
    }
    langs.extend(["040904b0".to_string(), "080404b0".to_string(), "040904e4".to_string()]);
    for lang in &langs {
        if info.description.is_none() {
            info.description = ver_string(&data, lang, "FileDescription");
        }
        if info.company.is_none() {
            info.company = ver_string(&data, lang, "CompanyName");
        }
        if info.product.is_none() {
            info.product = ver_string(&data, lang, "ProductName");
        }
        if info.version.is_none() {
            info.version = ver_string(&data, lang, "FileVersion");
        }
        if info.copyright.is_none() {
            info.copyright = ver_string(&data, lang, "LegalCopyright");
        }
        if info.original_name.is_none() {
            info.original_name = ver_string(&data, lang, "OriginalFilename");
        }
    }
    info
}

// ---------------------------------------------------------------------------
// 数字签名（使用 Get-AuthenticodeSignature，可正确识别目录签名的系统文件）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SignatureInfo {
    /// valid / notSigned / hashMismatch / notTrusted / unknownError / unavailable
    pub status: String,
    pub status_text: String,
    pub signer: Option<String>,
    pub issuer: Option<String>,
    pub signature_type: Option<String>,
    pub not_after: Option<String>,
    pub thumbprint: Option<String>,
}

pub fn signature(path: &str) -> SignatureInfo {
    let escaped = path.replace('\'', "''");
    let script = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;\
         $s=Get-AuthenticodeSignature -LiteralPath '{}';\
         $c=$s.SignerCertificate;\
         [pscustomobject]@{{Status=[string]$s.Status;Signer=if($c){{$c.Subject}}else{{$null}};\
         Issuer=if($c){{$c.Issuer}}else{{$null}};Type=[string]$s.SignatureType;\
         NotAfter=if($c){{$c.NotAfter.ToString('yyyy-MM-dd')}}else{{$null}};\
         Thumbprint=if($c){{$c.Thumbprint}}else{{$null}}}} | ConvertTo-Json -Compress",
        escaped
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let mut info = SignatureInfo {
        status: "unavailable".into(),
        status_text: "无法获取签名信息".into(),
        ..Default::default()
    };
    let out = match output {
        Ok(o) => o,
        Err(_) => return info,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = match serde_json::from_str(text.trim()) {
        Ok(v) => v,
        Err(_) => return info,
    };
    let gs = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string()).filter(|s| !s.is_empty());
    let status = gs("Status").unwrap_or_default();
    let (code, text) = match status.as_str() {
        "Valid" => ("valid", "签名有效"),
        "NotSigned" => ("notSigned", "文件未签名"),
        "HashMismatch" => ("hashMismatch", "签名无效：文件已被修改"),
        "NotTrusted" => ("notTrusted", "签名证书不受信任"),
        "NotSupportedFileFormat" => ("unavailable", "不支持的文件格式"),
        "Incompatible" => ("unavailable", "签名格式不兼容"),
        _ => ("unknownError", "签名验证失败"),
    };
    info.status = code.into();
    info.status_text = text.into();
    // 提取 CN
    let cn = |s: String| -> String {
        s.split(',')
            .find_map(|part| part.trim().strip_prefix("CN=").map(|x| x.trim_matches('"').to_string()))
            .unwrap_or(s)
    };
    info.signer = gs("Signer").map(cn);
    info.issuer = gs("Issuer").map(cn);
    info.signature_type = gs("Type").map(|t| match t.as_str() {
        "Catalog" => "目录签名 (Catalog)".to_string(),
        "Authenticode" => "嵌入签名 (Authenticode)".to_string(),
        other => other.to_string(),
    });
    info.not_after = gs("NotAfter");
    info.thumbprint = gs("Thumbprint");
    info
}

// ---------------------------------------------------------------------------
// Shell 操作
// ---------------------------------------------------------------------------

pub fn open_file_location(path: &str) -> Result<(), String> {
    if !std::path::Path::new(path).exists() {
        return Err("文件不存在或无权访问".into());
    }
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn show_file_properties(path: &str) -> Result<(), String> {
    if !std::path::Path::new(path).exists() {
        return Err("文件不存在或无权访问".into());
    }
    let path = path.to_string();
    std::thread::spawn(move || unsafe {
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let verb = wide("properties");
        let file = wide(&path);
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_INVOKEIDLIST;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.nShow = SW_SHOWNORMAL;
        ShellExecuteExW(&mut sei);
        // 属性窗口依附于调用线程的消息循环，保持线程存活一段时间
        use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE};
        let start = std::time::Instant::now();
        let mut msg: MSG = std::mem::zeroed();
        while start.elapsed().as_secs() < 600 {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
    Ok(())
}

/// 当前进程是否以管理员身份运行
pub fn is_elevated() -> bool {
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev: TOKEN_ELEVATION = std::mem::zeroed();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        windows_sys::Win32::Foundation::CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// 以管理员身份重新启动 FanGuard（仅在用户明确请求时调用）
pub fn relaunch_as_admin() -> Result<(), String> {
    relaunch_as_admin_with("--elevated")
}

pub fn relaunch_as_admin_with(params: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(params);
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (r as isize) > 32 {
        Ok(())
    } else {
        Err("用户取消了管理员权限请求".into())
    }
}
