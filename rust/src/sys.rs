// WorkBuddy2API · 系统层
// 路径定位 / config.json / 网关进程 / 端口占用 / healthz 探活 / 日志 tail
// / 账号列举 / 开机自启 / 局域网 IP

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const DETACHED_PROCESS: u32 = 0x0000_0008;

// ────────────────────────── 路径 ──────────────────────────

static DATA_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|x| x.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_data_root(args: &[String], local: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(index) = args.iter().position(|arg| arg == "--data-dir") {
        let path = args.get(index + 1).map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or("--data-dir 需要一个完整的目录路径")?;
        return Ok(path);
    }
    local.map(|path| path.join("WorkBuddy2API"))
        .ok_or_else(|| "无法定位当前用户的 LocalAppData 目录".into())
}

pub fn initialize_paths(args: &[String]) -> Result<(), String> {
    let path = resolve_data_root(args, std::env::var_os("LOCALAPPDATA").map(PathBuf::from))?;
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    DATA_ROOT.set(path).map_err(|_| "运行数据目录已经初始化".to_string())
}

/// Configuration and mutable data always belong to the current user, independent of EXE location.
pub fn root() -> PathBuf {
    DATA_ROOT.get().cloned().unwrap_or_else(|| {
        resolve_data_root(&[], std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
            .expect("LocalAppData must be available")
    })
}

pub fn config_path() -> PathBuf {
    root().join("config.json")
}
pub fn auths_dir() -> PathBuf {
    root().join("auths")
}
pub fn data_dir() -> PathBuf {
    root().join("data")
}
pub fn logs_dir() -> PathBuf {
    root().join("logs")
}
pub fn gw_log() -> PathBuf {
    logs_dir().join("gateway.log")
}
pub fn state_path() -> PathBuf {
    data_dir().join("panel_state.json")
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ────────────────────────── 配置 ──────────────────────────

pub const DEFAULT_CONFIG: &str = r#"{
  "listen": ":8080",
  "api_key": "",
  "auth_dir": "./auths",
  "state_file": "./data/state.json",
  "server": { "max_body_mb": 8 },
  "cooldown": { "soft_rate": "600s", "soft_rate_max": "2h" },
  "schedule": {
    "checkin_hours": [9, 21], "travel_hours": [9, 21], "activity_hours": [10],
    "keepalive_hours": [22], "school_hours": [12], "cat_hours": [1],
    "checkin_enabled": true, "travel_enabled": true, "activity_enabled": true,
    "keepalive_enabled": true,
    "school_enabled": false, "cat_enabled": false
  },
  "global": { "enabled": true, "chat_base": "", "billing_base": "" },
  "free_gateways": { "opencode": { "enabled": true }, "kilo": { "enabled": true } },
  "upstream": {
    "timeout_seconds": 120, "header_timeout_seconds": 120, "idle_timeout_seconds": 300,
    "user_agent": "", "client_version": "", "cli_version": "",
    "device_token": "", "device_token_file": "", "client_name": "WorkBuddy",
    "passthrough_ip": false
  },
  "features": { "sanitize_blacklist_fingerprints": true },
  "prompt": { "mode": "passthrough", "file": "" },
  "upstash": { "url": "", "token": "" },
  "pool": {
    "max_in_flight": 3, "breaker_threshold": 3, "breaker_cooldown": "30m",
    "breaker_cooldown_max": "6h", "idle_weight_per_hour": 0.5,
    "idle_weight_max": 5.0, "expiring_soon": "168h"
  },
  "session_sticky": { "enabled": true, "ttl": "30m", "gc_interval": "5m" }
}"#;

pub fn default_config() -> Value {
    serde_json::from_str(DEFAULT_CONFIG).unwrap_or_else(|_| json!({}))
}

pub fn read_config() -> Value {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(default_config)
}

pub fn write_config(cfg: &Value) -> Result<(), String> {
    if let Some(p) = config_path().parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(config_path(), s).map_err(|e| e.to_string())
}

pub fn cfg_port(cfg: &Value) -> u16 {
    cfg.get("listen")
        .and_then(|v| v.as_str())
        .unwrap_or(":8080")
        .rsplit(':')
        .next()
        .unwrap_or("")
        .parse::<u16>()
        .unwrap_or(8080)
}

pub fn api_key(cfg: &Value) -> String {
    cfg.get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub const FREE_GATEWAY_IDS: [&str; 2] = ["opencode", "kilo"];

/// An omitted switch keeps the gateway's default enabled behavior on older profiles.
pub fn free_gateway_settings(cfg: &Value) -> [bool; 2] {
    FREE_GATEWAY_IDS.map(|id| {
        cfg.get("free_gateways").and_then(|v| v.get(id))
            .and_then(|v| v.get("enabled")).and_then(Value::as_bool).unwrap_or(true)
    })
}

/// Only edit the two switches; preserve unrelated and future provider options.
pub fn merge_free_gateway_settings(cfg: &mut Value, enabled: [bool; 2]) -> Result<(), String> {
    if !cfg.is_object() {
        return Err("config.json 顶层必须是 JSON 对象".into());
    }
    if let Some(gateways) = cfg.get("free_gateways") {
        if !gateways.is_object() {
            return Err("free_gateways 必须是 JSON 对象，请先修复配置".into());
        }
        for id in FREE_GATEWAY_IDS {
            if gateways.get(id).is_some_and(|provider| !provider.is_object()) {
                return Err(format!("free_gateways.{id} 必须是 JSON 对象，请先修复配置"));
            }
        }
    } else {
        cfg["free_gateways"] = json!({});
    }
    for (id, value) in FREE_GATEWAY_IDS.into_iter().zip(enabled) {
        if cfg["free_gateways"].get(id).is_none() {
            cfg["free_gateways"][id] = json!({});
        }
        cfg["free_gateways"][id]["enabled"] = Value::Bool(value);
    }
    Ok(())
}

pub fn save_free_gateway_settings(enabled: [bool; 2]) -> Result<(), String> {
    // Do not let the permissive display reader replace an invalid existing profile.
    let mut cfg = match fs::read_to_string(config_path()) {
        Ok(source) => serde_json::from_str(&source)
            .map_err(|error| format!("config.json 无法解析，未覆盖原文件: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => default_config(),
        Err(error) => return Err(format!("无法读取 config.json: {error}")),
    };
    merge_free_gateway_settings(&mut cfg, enabled)?;
    write_config(&cfg)
}

/// 内置的网关二进制：编译期打进本 exe，让交付物真正只剩一个文件。
/// 路径相对本源码文件（rust/src/）：../../ = 项目根目录
pub const EMBEDDED_GATEWAY: &[u8] = include_bytes!("../../wb2api.exe");

/// Installed gateway lives beside the app; single-file copies extract only into the data directory.
pub fn ensure_gateway() -> Option<PathBuf> {
    let local = exe_dir().join("wb2api.exe");
    if local.is_file() {
        return Some(local);
    }
    let dir = root().join("bin");
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join("wb2api.exe");
    if fs::read(&path).ok().as_deref() != Some(EMBEDDED_GATEWAY) {
        fs::write(&path, EMBEDDED_GATEWAY).ok()?;
    }
    Some(path)
}

/// Installer shutdown never terminates an unrelated process that happens to own the port or PID.
pub fn shutdown_owned_gateway() -> Result<(), String> {
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::Foundation::CloseHandle;
    let allowed: Vec<_> = [exe_dir().join("wb2api.exe"), root().join("bin/wb2api.exe")]
        .iter().filter_map(|path| fs::canonicalize(path).ok()).collect();
    let mut pids = pids_on_port(cfg_port(&read_config()));
    if let Some(pid) = read_state().get("pid").and_then(Value::as_u64) {
        if !pids.contains(&(pid as u32)) { pids.push(pid as u32); }
    }
    for pid in pids {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() { continue; }
            let mut name = vec![0u16; 32768];
            let mut length = name.len() as u32;
            let queried = QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut length) != 0;
            let owned = queried && fs::canonicalize(PathBuf::from(String::from_utf16_lossy(&name[..length as usize])))
                .ok().map(|path| allowed.contains(&path)).unwrap_or(false);
            let stopped = !owned || (TerminateProcess(process, 0) != 0 && WaitForSingleObject(process, 5000) == 0);
            CloseHandle(process);
            if !stopped { return Err("无法停止网关，请从托盘停止服务后重试".into()); }
        }
    }
    Ok(())
}

pub fn find_exe() -> Option<PathBuf> {
    ensure_gateway()
}

// ────────────────────────── 进程 / 端口 ──────────────────────────

fn run_cmd(program: &str, args: &[&str]) -> Option<std::process::Output> {
    let mut c = Command::new(program);
    c.args(args);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c.output().ok()
}

pub fn pids_on_port(port: u16) -> Vec<u32> {
    let out = match run_cmd("netstat", &["-ano"]) {
        Some(o) => o,
        None => return Vec::new(),
    };
    let s = String::from_utf8_lossy(&out.stdout);
    let needle = format!(":{} ", port);
    let mut v: Vec<u32> = Vec::new();
    for line in s.lines() {
        if line.contains(&needle) && line.contains("LISTENING") {
            if let Some(Ok(p)) = line.split_whitespace().last().map(|x| x.parse::<u32>()) {
                if !v.contains(&p) {
                    v.push(p);
                }
            }
        }
    }
    v
}

pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    match run_cmd("tasklist", &["/fi", &format!("PID eq {}", pid), "/nh"]) {
        Some(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
        None => false,
    }
}

pub fn kill_pid(pid: u32) -> bool {
    run_cmd("taskkill", &["/f", "/pid", &pid.to_string()])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// healthz 探活：只要拿到 HTTP 响应就算通（网关在 auths/ 为空时返回 503 属正常）。
pub fn probe_health(port: u16, timeout: Duration) -> (bool, String) {
    let addr: SocketAddr = match format!("127.0.0.1:{}", port).parse() {
        Ok(a) => a,
        Err(_) => return (false, "端口非法".into()),
    };
    let mut s = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(e) => return (false, e.to_string()),
    };
    let _ = s.set_read_timeout(Some(timeout));
    let req = format!("GET /healthz HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", port);
    if let Err(e) = s.write_all(req.as_bytes()) {
        return (false, e.to_string());
    }
    let mut buf = vec![0u8; 8192];
    match s.read(&mut buf) {
        Ok(0) => (false, "空响应".into()),
        Ok(n) => {
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            let first = text.lines().next().unwrap_or("").trim().to_string();
            (true, if first.is_empty() { "HTTP OK".into() } else { first })
        }
        Err(e) => (false, e.to_string()),
    }
}

// ────────────────────────── 运行时状态 ──────────────────────────

pub fn read_state() -> Value {
    fs::read_to_string(state_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn write_state(v: &Value) -> Result<(), String> {
    fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let s = serde_json::to_string(v).map_err(|e| e.to_string())?;
    fs::write(state_path(), s).map_err(|e| e.to_string())
}

// ────────────────────────── 日志 ──────────────────────────

pub fn tail_log(n: usize) -> String {
    let p = gw_log();
    let mut f = match fs::File::open(&p) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = size.saturating_sub(60_000);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    let s = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = s.lines().collect();
    let take = lines.len().saturating_sub(n);
    lines[take..].join("\n")
}

// ────────────────────────── 账号 ──────────────────────────

#[derive(Clone, Default, Debug)]
pub struct Account {
    pub file: String,
    pub uid: String,
    pub nick: String,
    pub realm: String,
    pub exp: String,
}

fn pick_str(v: &Value, alt: &Value, keys: &[&str]) -> String {
    for k in keys {
        for src in [alt, v] {
            match src.get(*k) {
                Some(Value::String(s)) if !s.is_empty() => return s.clone(),
                Some(Value::Number(n)) => return n.to_string(),
                _ => {}
            }
        }
    }
    String::new()
}

pub fn list_accounts() -> Vec<Account> {
    let names = match fs::read_dir(auths_dir()) {
        Ok(rd) => {
            let mut v: Vec<String> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.to_lowercase().ends_with(".json"))
                .collect();
            v.sort();
            v
        }
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for file in names {
        let mut a = Account {
            file: file.clone(),
            uid: "?".into(),
            ..Default::default()
        };
        if let Ok(s) = fs::read_to_string(auths_dir().join(&file)) {
            if let Ok(d) = serde_json::from_str::<Value>(&s) {
                // 新版凭证是嵌套结构 {account:{...}, auth:{...}}，旧版可能是平铺
                let acc = d.get("account").cloned().unwrap_or_else(|| json!({}));
                let auth = d.get("auth").cloned().unwrap_or_else(|| json!({}));
                let uid = pick_str(&d, &acc, &["uid", "user_id"]);
                a.uid = if uid.is_empty() { "?".into() } else { uid };
                a.nick = pick_str(&d, &acc, &["nickname", "nick"]);
                a.realm = pick_str(&d, &auth, &["realm", "domain"]);

                let ts = auth
                    .get("expiresAt")
                    .and_then(|x| x.as_i64())
                    .or_else(|| d.get("expires_at").and_then(|x| x.as_i64()))
                    .or_else(|| d.get("expired_at").and_then(|x| x.as_i64()))
                    .or_else(|| d.get("exp").and_then(|x| x.as_i64()))
                    .unwrap_or(0);
                if ts > 1_000_000_000 {
                    let left = ts - now_secs();
                    a.exp = if left > 0 {
                        format!("{:.1} 小时", left as f64 / 3600.0)
                    } else {
                        "已过期".into()
                    };
                }
            }
        }
        out.push(a);
    }
    out
}

pub fn delete_account(file: &str) -> Result<(), String> {
    let p = auths_dir().join(file);
    fs::remove_file(&p).map_err(|e| e.to_string())
}

// ────────────────────────── 杂项 ──────────────────────────

pub fn gen_key(n: usize) -> String {
    const CH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = now_secs() as u64 ^ 0x9E3779B97F4A7C15;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            CH[(s as usize) % CH.len()] as char
        })
        .collect()
}

/// 探测本机对外局域网 IP（UDP connect 不发包，只取路由出口地址）。
pub fn lan_ip() -> String {
    use std::net::UdpSocket;
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                return a.ip().to_string();
            }
        }
    }
    "127.0.0.1".into()
}

pub fn startup_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join("WorkBuddy2API-Autostart.vbs"),
    )
}

pub fn startup_on() -> bool {
    startup_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn set_startup(on: bool) -> Result<(), String> {
    let p = startup_path().ok_or_else(|| "无法定位启动目录".to_string())?;
    if on {
        let exe = find_exe().ok_or_else(|| "找不到 wb2api.exe".to_string())?;
        let body = format!(
            "Set ws = CreateObject(\"WScript.Shell\")\r\n\
             ws.CurrentDirectory = \"{}\"\r\n\
             ws.Run \"\"{}\"\" -config \"\"{}\"\"\", 0, False\r\n",
            root().display(),
            exe.display(),
            config_path().display()
        );
        fs::write(&p, body).map_err(|e| e.to_string())?;
    } else if p.exists() {
        fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn open_in_explorer(path: &Path) {
    let _ = fs::create_dir_all(path);
    let mut c = Command::new("explorer");
    c.arg(path);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    let _ = c.spawn();
}

pub fn open_url(url: &str) {
    // Pass one complete URL directly to Windows. `cmd /c start` interprets '&'
    // in OAuth query strings as another command and silently drops parameters.
    if url.contains('\0') {
        return;
    }
    #[cfg(windows)]
    {
        let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        let target: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(), operation.as_ptr(), target.as_ptr(),
                std::ptr::null(), std::ptr::null(),
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            );
        }
    }
    #[cfg(not(windows))]
    { let _ = Command::new("xdg-open").arg(url).spawn(); }
}

// ────────────────────────── 网关启停 ──────────────────────────

pub fn start_gateway() -> Result<(u32, bool), String> {
    let exe = find_exe().ok_or_else(|| "找不到 wb2api.exe".to_string())?;
    let cfg = read_config();
    let port = cfg_port(&cfg);

    // 清掉端口占用与上一次的残留进程
    let st = read_state();
    let old = st.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if old > 0 && pid_alive(old) && pids_on_port(port).is_empty() {
        kill_pid(old);
    }
    for p in pids_on_port(port) {
        kill_pid(p);
    }
    std::thread::sleep(Duration::from_millis(600));

    for d in [auths_dir(), data_dir(), logs_dir()] {
        fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    }
    if !config_path().exists() {
        write_config(&cfg)?;
    }

    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(gw_log())
        .map_err(|e| format!("打开日志失败: {}", e))?;
    let f2 = f
        .try_clone()
        .map_err(|e| format!("复制日志句柄失败: {}", e))?;

    let child = {
        let mut c = Command::new(&exe);
        c.args(["-config", &config_path().to_string_lossy()])
            .current_dir(root())
            .stdout(Stdio::from(f))
            .stderr(Stdio::from(f2))
            .stdin(Stdio::null());
        #[cfg(windows)]
        c.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
        c.spawn().map_err(|e| format!("启动失败: {}", e))?
    };

    let pid = child.id();
    let started_ts = now_secs();
    let _ = write_state(&json!({
        "pid": pid, "mode": "hidden", "started_ts": started_ts, "port": port
    }));

    // 轮询探活，最多约 5 秒
    let mut ok = false;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(500));
        let (h, _) = probe_health(port, Duration::from_millis(1200));
        if h {
            ok = true;
            break;
        }
    }
    Ok((pid, ok))
}

pub fn stop_gateway() -> Vec<String> {
    let port = cfg_port(&read_config());
    let mut killed: Vec<String> = Vec::new();
    for p in pids_on_port(port) {
        if kill_pid(p) {
            killed.push(p.to_string());
        }
    }
    let st = read_state();
    let sp = st.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if sp > 0 && !killed.iter().any(|x| x == &sp.to_string()) && pid_alive(sp) {
        if kill_pid(sp) {
            killed.push(sp.to_string());
        }
    }
    let _ = write_state(&json!({}));
    killed
}

// ────────────────────────── 后台采集 ──────────────────────────

#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct Collected {
    pub port: u16,
    pub pids: Vec<u32>,
    pub pid: Option<u32>,
    pub running: bool,
    pub health: bool,
    pub health_text: String,
    pub started_ts: i64,
    pub accounts: Vec<Account>,
    pub log: String,
}

pub fn collect() -> Collected {
    let cfg = read_config();
    let port = cfg_port(&cfg);
    let st = read_state();
    let pids = pids_on_port(port);
    let (health, health_text) = probe_health(port, Duration::from_millis(1500));
    let spid = st.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32);
    let pid = if !pids.is_empty() {
        pids.first().copied()
    } else {
        spid.filter(|p| pid_alive(*p))
    };
    Collected {
        port,
        running: !pids.is_empty(),
        pids,
        pid,
        health,
        health_text,
        started_ts: st.get("started_ts").and_then(|v| v.as_i64()).unwrap_or(0),
        accounts: list_accounts(),
        log: tail_log(120),
    }
}

// ────────────────────────── 开机自启（界面） ──────────────────────────

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| root().join("WorkBuddy2API.exe"))
}

fn startup_dir_file(name: &str) -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(name),
    )
}

pub fn gui_startup_path() -> Option<PathBuf> {
    startup_dir_file("WorkBuddy2API-GUI.vbs")
}

pub fn gui_startup_on() -> bool {
    gui_startup_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn set_gui_startup(on: bool) -> Result<(), String> {
    let p = gui_startup_path().ok_or_else(|| "无法定位启动目录".to_string())?;
    if on {
        let exe = exe_path();
        let body = format!(
            "Set ws = CreateObject(\"WScript.Shell\")\r\n\
             ws.CurrentDirectory = \"{}\"\r\n\
             ws.Run \"\"{}\"\" --minimized\", 0, False\r\n",
            root().display(),
            exe.display()
        );
        fs::write(&p, body).map_err(|e| e.to_string())?;
    } else if p.exists() {
        fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ────────────────────────── Token 用量（解析网关日志） ──────────────────────────
// 网关把每次请求记成这样的表格行：
// | #001 | 01:13:30 | global:deep | stream | 200 | uid=1a2b3c4d | TTFB=1141ms | tok=106 | 65.4tok/s | total=1.6s |
// /status 只给账号健康、/metrics 是 404，所以用量只能从日志算。

#[derive(Clone, Default, Debug)]
pub struct UsageStats {
    pub total_req: u64,
    pub ok_req: u64,
    pub total_tokens: u64,
    pub today_req: u64,
    pub today_tokens: u64,
    pub today_label: String,
    pub by_model: Vec<(String, u64, u64)>, // (模型, tokens, 次数)
    pub by_uid: Vec<(String, u64, u64)>,   // (账号, tokens, 次数)
    pub avg_tps: f32,
    pub avg_ttfb_ms: f32,
    pub live_instance: String,
    pub live_baseline_tokens: u64,
}

fn digits_after(s: &str, key: &str) -> u64 {
    let s = s.trim();
    let s = match s.find(key) {
        Some(p) => &s[p + key.len()..],
        None => s,
    };
    let num: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().unwrap_or(0)
}

fn float_of(s: &str) -> f64 {
    let num: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().unwrap_or(0.0)
}

/// 日志里最后一次出现的日期，当作"今天"（避免手算时区）。
fn last_log_date(text: &str) -> String {
    let mut d = String::new();
    for line in text.lines() {
        let l = line.trim_end();
        if l.len() >= 10 && l.as_bytes().get(4) == Some(&b'/') && l.as_bytes().get(7) == Some(&b'/')
        {
            if l[..10].chars().all(|c| c.is_ascii_digit() || c == '/') {
                d = l[..10].to_string();
            }
        }
    }
    d
}

pub fn parse_usage_from(path: &Path) -> UsageStats {
    let mut st = UsageStats::default();
    let Ok(data) = fs::read(path) else {
        return st;
    };
    let text = String::from_utf8_lossy(&data).to_string();
    let today = last_log_date(&text);
    st.today_label = today.clone();

    let mut cur_date = String::new();
    let mut model_map: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut uid_map: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut tps_sum = 0f64;
    let mut tps_n = 0u64;
    let mut ttfb_sum = 0u64;
    let mut ttfb_n = 0u64;

    for line in text.lines() {
        let l = line.trim_end();
        if let Some((_, instance)) = l.split_once("live usage instance=") {
            st.live_instance = instance.trim().to_string();
            st.live_baseline_tokens = st.total_tokens;
        }
        if l.len() >= 10 && l.as_bytes().get(4) == Some(&b'/') && l.as_bytes().get(7) == Some(&b'/')
        {
            if l[..10].chars().all(|c| c.is_ascii_digit() || c == '/') {
                cur_date = l[..10].to_string();
            }
            continue;
        }
        if !l.starts_with('|') {
            continue;
        }
        let parts: Vec<&str> = l.split('|').collect();
        if parts.len() < 11 {
            continue;
        }
        let status: u64 = parts[5].trim().parse().unwrap_or(0);
        let model = parts[3].trim().to_string();
        let uid = parts[6].trim().trim_start_matches("uid=").to_string();
        let ttfb = digits_after(parts[7], "TTFB=");
        let tok = digits_after(parts[8], "tok=");
        let tps = float_of(parts[9]);
        if model.is_empty() {
            continue;
        }

        st.total_req += 1;
        st.total_tokens += tok;
        if status == 200 {
            st.ok_req += 1;
        }
        if !cur_date.is_empty() && cur_date == today {
            st.today_req += 1;
            st.today_tokens += tok;
        }
        let e = model_map.entry(model).or_insert((0, 0));
        e.0 += tok;
        e.1 += 1;
        let e = uid_map.entry(uid).or_insert((0, 0));
        e.0 += tok;
        e.1 += 1;
        // 均值按"全部请求"计算，不要跳过 tok=0 的请求，否则会把平均速度抬高
        tps_sum += tps;
        tps_n += 1;
        ttfb_sum += ttfb;
        ttfb_n += 1;
    }

    let mut mv: Vec<(String, u64, u64)> = model_map
        .into_iter()
        .map(|(k, (t, c))| (k, t, c))
        .collect();
    mv.sort_by(|a, b| b.1.cmp(&a.1));
    let mut uv: Vec<(String, u64, u64)> =
        uid_map.into_iter().map(|(k, (t, c))| (k, t, c)).collect();
    uv.sort_by(|a, b| b.1.cmp(&a.1));
    st.by_model = mv;
    st.by_uid = uv;
    st.avg_tps = if tps_n > 0 {
        (tps_sum / tps_n as f64) as f32
    } else {
        0.0
    };
    st.avg_ttfb_ms = if ttfb_n > 0 {
        (ttfb_sum as f64 / ttfb_n as f64) as f32
    } else {
        0.0
    };
    st
}

pub fn parse_usage() -> UsageStats {
    parse_usage_from(&gw_log())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_directory_is_independent_of_executable_location() {
        let local = PathBuf::from(r"C:\Users\Example\AppData\Local");
        assert_eq!(resolve_data_root(&[], Some(local.clone())).unwrap(), local.join("WorkBuddy2API"));
        let args = vec!["app.exe".into(), "--data-dir".into(), r"C:\Temp\isolated profile".into()];
        assert_eq!(resolve_data_root(&args, Some(local)).unwrap(), PathBuf::from(&args[2]));
        assert!(resolve_data_root(&["--data-dir".into()], None).is_err());
        assert!(resolve_data_root(&["--data-dir".into(), "relative".into()], None).is_err());
        assert!(resolve_data_root(&[], None).is_err());
    }

    #[test]
    fn default_config_is_valid_and_8080() {
        let c = default_config();
        assert_eq!(cfg_port(&c), 8080);
        for k in ["schedule", "pool", "session_sticky", "upstream", "global"] {
            assert!(c.get(k).is_some(), "默认配置缺少字段 {}", k);
        }
        assert_eq!(api_key(&c), "");
    }

    #[test]
    fn embedded_gateway_is_a_valid_pe() {
        // 内置网关必须是合法 PE（MZ 头），且体积合理（约 12MB）
        assert!(
            EMBEDDED_GATEWAY.len() > 1_000_000,
            "内置网关体积异常: {} 字节",
            EMBEDDED_GATEWAY.len()
        );
        assert_eq!(&EMBEDDED_GATEWAY[..2], b"MZ", "内置网关不是合法 PE 文件");
    }

    #[test]
    fn cfg_port_parses_listen() {
        assert_eq!(cfg_port(&json!({"listen": ":9000"})), 9000);
        assert_eq!(cfg_port(&json!({"listen": "8081"})), 8081);
        assert_eq!(cfg_port(&json!({})), 8080, "缺 listen 应回退 8080");
        assert_eq!(cfg_port(&json!({"listen": "garbage"})), 8080);
    }

    #[test]
    fn free_gateway_switches_default_on_and_preserve_explicit_off() {
        assert_eq!(free_gateway_settings(&json!({})), [true, true]);
        assert_eq!(free_gateway_settings(&json!({"free_gateways": {"opencode": {"enabled": false}}})), [false, true]);
        assert_eq!(free_gateway_settings(&json!({"free_gateways": {"kilo": {"enabled": false}}})), [true, false]);
        assert_eq!(free_gateway_settings(&default_config()), [true, true]);
    }

    #[test]
    fn free_gateway_merge_preserves_other_options_and_rejects_invalid_shapes() {
        let mut cfg = json!({
            "api_key": "synthetic-test-key", "custom": {"future": 42},
            "free_gateways": {"opencode": {"enabled": true, "future_option": "keep"},
                "another_provider": {"enabled": false}}
        });
        merge_free_gateway_settings(&mut cfg, [false, true]).unwrap();
        assert_eq!(free_gateway_settings(&cfg), [false, true]);
        assert_eq!(cfg["api_key"], "synthetic-test-key");
        assert_eq!(cfg["custom"]["future"], 42);
        assert_eq!(cfg["free_gateways"]["opencode"]["future_option"], "keep");
        assert_eq!(cfg["free_gateways"]["another_provider"]["enabled"], false);
        let mut absent = json!({"listen": ":9000"});
        merge_free_gateway_settings(&mut absent, [true, false]).unwrap();
        assert_eq!(free_gateway_settings(&absent), [true, false]);
        for mut invalid in [json!([]), json!({"free_gateways": false}),
            json!({"free_gateways": {"opencode": null}}),
            json!({"free_gateways": {"kilo": "do not erase"}})] {
            let original = invalid.clone();
            assert!(merge_free_gateway_settings(&mut invalid, [true, true]).is_err());
            assert_eq!(invalid, original, "validation must finish before any mutation");
        }
    }

    #[test]
    fn gen_key_shape() {
        let a = gen_key(24);
        assert_eq!(a.chars().count(), 24);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(gen_key(8).chars().count(), 8);
    }

    #[test]
    fn lan_ip_not_empty() {
        let ip = lan_ip();
        assert!(!ip.is_empty());
        assert!(ip.contains('.'), "应为 IPv4 地址，实际 {}", ip);
    }

    #[test]
    fn collect_does_not_panic_and_has_port() {
        let c = collect();
        assert!(c.port > 0);
        // running 与 pids 必须自洽
        assert_eq!(c.running, !c.pids.is_empty());
    }

    #[test]
    fn pids_on_port_returns_no_error_on_unused_port() {
        // 用一个几乎不可能被占用的高位端口
        let v = pids_on_port(45671);
        assert!(v.is_empty() || v.iter().all(|p| *p > 0));
    }

    #[test]
    fn probe_health_on_closed_port_is_false() {
        let (ok, _) = probe_health(45671, Duration::from_millis(300));
        assert!(!ok);
    }

    #[test]
    fn usage_helpers_parse_numbers() {
        assert_eq!(digits_after(" TTFB=1141ms ", "TTFB="), 1141);
        assert_eq!(digits_after(" tok=106 ", "tok="), 106);
        assert!((float_of(" 65.4tok/s ") - 65.4).abs() < 1e-6);
        assert_eq!(digits_after(" tok=0 ", "tok="), 0);
    }

    #[test]
    fn parse_usage_parses_synthetic_log() {
        let dir = std::env::temp_dir().join("w2a_usage_test");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("gateway.log");
        let log = "2026/10/15 01:11:10 listening on :8080\n\
| #001 | 01:13:30 | global:deep | stream | 200 | uid=abc | TTFB=1141ms | tok=106 | 65.4tok/s | total=1.6s |\n\
| #002 | 01:14:27 | global:deep | stream | 200 | uid=abc | TTFB=2285ms | tok=294 | 85.4tok/s | total=3.5s |\n\
| #003 | 01:15:01 | cn:fast | stream | 500 | uid=xyz | TTFB=100ms | tok=10 | 5.0tok/s | total=0.2s |\n";
        fs::write(&p, log).unwrap();

        let u = parse_usage_from(&p);
        assert_eq!(u.total_req, 3);
        assert_eq!(u.ok_req, 2, "只有前两条是 200");
        assert_eq!(u.total_tokens, 410);
        assert_eq!(u.today_tokens, 410, "同一天，今日应等于累计");
        assert_eq!(u.today_label, "2026/10/15");
        assert_eq!(u.by_model.len(), 2);
        assert_eq!(u.by_model[0], ("global:deep".to_string(), 400, 2));
        assert_eq!(u.by_model[1], ("cn:fast".to_string(), 10, 1));
        assert_eq!(u.by_uid.len(), 2);
        assert_eq!(u.by_uid[0], ("abc".to_string(), 400, 2));
        assert!((u.avg_tps - 51.93).abs() < 0.1);
        assert!((u.avg_ttfb_ms - 1175.33).abs() < 1.0);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn parse_usage_does_not_panic_and_is_consistent() {
        let u = parse_usage();
        // 分组求和必须等于总量
        let sum: u64 = u.by_model.iter().map(|(_, t, _)| *t).sum();
        assert_eq!(sum, u.total_tokens, "按模型分组的 token 之和应等于总 token");
        assert!(u.ok_req <= u.total_req);
    }

    #[test]
    fn live_startup_marker_preserves_history_across_gateway_restarts() {
        let p = std::env::temp_dir().join(format!("w2a-live-marker-{}.log", std::process::id()));
        fs::write(&p, "2026/10/15 10:00:00 startup\n\
| #001 | 10:00:01 | cn:hy3 | stream | 200 | uid=fixture | TTFB=10ms | tok=100 | 10tok/s | total=10s |\n\
2026/10/15 11:00:00 live usage instance=first\n\
| #001 | 11:00:01 | cn:hy3 | stream | 200 | uid=fixture | TTFB=10ms | tok=25 | 10tok/s | total=2s |\n\
2026/10/15 12:00:00 live usage instance=second\n\
| #001 | 12:00:01 | cn:hy3 | stream | 200 | uid=fixture | TTFB=10ms | tok=30 | 10tok/s | total=3s |\n").unwrap();
        let u = parse_usage_from(&p);
        assert_eq!(u.live_instance, "second");
        assert_eq!((u.live_baseline_tokens, u.total_tokens), (125, 155));
        let _ = fs::remove_file(p);
    }

    #[test]
    fn state_roundtrip_uses_json() {
        let v = json!({"pid": 1234, "started_ts": 1_700_000_000_i64});
        assert_eq!(v.get("pid").and_then(|x| x.as_u64()), Some(1234));
        assert_eq!(v.get("started_ts").and_then(|x| x.as_i64()), Some(1_700_000_000));
    }
}
