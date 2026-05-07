mod tui;

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    fs::File,
    io::Write,
    net::IpAddr,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::{debug, error, info, warn};
use user_idle::UserIdle;
use warp::Filter;

#[derive(Parser)]
#[command(name = "pushel", about = "Desktop notification reminder")]
struct Cli {
    #[arg(short, long, help = "Launch TUI notification manager")]
    tui: bool,
}

const VALID_URGENCIES: &[&str] = &["low", "normal", "critical"];
const MAX_FIELD_LENGTH: usize = 1024;
const MAX_MESSAGE_LENGTH: usize = 4096;

#[derive(Debug, Deserialize)]
pub(crate) struct NotificationConfig {
    title: Option<String>,
    message: String,
    interval: String,
    urgency: Option<String>,
    expire_time: Option<u32>,
    app_name: Option<String>,
    icon: Option<String>,
    category: Option<String>,
    transient: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AdhocNotification {
    title: Option<String>,
    message: String,
    urgency: Option<String>,
    expire_time: Option<u32>,
    app_name: Option<String>,
    icon: Option<String>,
    category: Option<String>,
    transient: Option<bool>,
}

impl From<AdhocNotification> for NotificationConfig {
    fn from(notif: AdhocNotification) -> Self {
        NotificationConfig {
            title: notif.title,
            message: notif.message,
            interval: String::new(),
            urgency: notif.urgency,
            expire_time: notif.expire_time,
            app_name: notif.app_name,
            icon: notif.icon,
            category: notif.category,
            transient: notif.transient,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AppConfig {
    listen_address: String,
    port: u16,
    webserver_enabled: bool,
    log_format: String,
    #[serde(default)]
    api_token: Option<String>,
    #[serde(default = "default_rate_limit_rpm")]
    rate_limit_rpm: u32,
    homeassistant_url: Option<String>,
    homeassistant_api_key: Option<String>,
}

fn default_rate_limit_rpm() -> u32 {
    60
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
enum MotionStatus {
    Active,
    Inactive,
}

impl MotionStatus {
    fn as_str(&self) -> &str {
        match self {
            MotionStatus::Active => "active",
            MotionStatus::Inactive => "inactive",
        }
    }
}

struct MotionTracker {
    last_motion: Arc<Mutex<Option<Instant>>>,
    current_status: Arc<Mutex<MotionStatus>>,
    runtime_handle: tokio::runtime::Handle,
}

impl MotionTracker {
    fn new(runtime_handle: tokio::runtime::Handle) -> Self {
        MotionTracker {
            last_motion: Arc::new(Mutex::new(None)),
            current_status: Arc::new(Mutex::new(MotionStatus::Inactive)),
            runtime_handle,
        }
    }

    fn update_motion(&self) {
        if let Ok(mut last_motion) = self.last_motion.lock() {
            *last_motion = Some(Instant::now());
        }
    }

    fn should_notify(&self) -> bool {
        if let Ok(last_motion) = self.last_motion.lock() {
            if let Some(last_motion_time) = *last_motion {
                return Instant::now().duration_since(last_motion_time)
                    <= Duration::from_secs(15 * 60);
            }
        }
        false
    }

    fn update_status(
        &self,
        new_status: MotionStatus,
        ha_url: Option<&str>,
        ha_api_key: Option<&str>,
    ) {
        let should_push = {
            if let Ok(mut current_status) = self.current_status.lock() {
                if *current_status != new_status {
                    info!(
                        "Motion status changed: {:?} -> {:?}",
                        *current_status, new_status
                    );
                    *current_status = new_status;
                    ha_url.is_some() && ha_api_key.is_some()
                } else {
                    false
                }
            } else {
                false
            }
        };

        if should_push {
            let url = ha_url.unwrap().to_string();
            let api_key = ha_api_key.unwrap().to_string();
            self.runtime_handle.spawn(async move {
                if let Err(e) = push_to_homeassistant(&url, &api_key, new_status).await {
                    error!("Failed to push status to Home Assistant: {}", e);
                }
            });
        }
    }
}

impl Clone for MotionTracker {
    fn clone(&self) -> Self {
        MotionTracker {
            last_motion: Arc::clone(&self.last_motion),
            current_status: Arc::clone(&self.current_status),
            runtime_handle: self.runtime_handle.clone(),
        }
    }
}

struct RateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, (Instant, u32)>>>,
    max_requests: u32,
    window: Duration,
}

impl RateLimiter {
    fn new(rpm: u32) -> Self {
        RateLimiter {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_requests: rpm,
            window: Duration::from_secs(60),
        }
    }

    fn check(&self, ip: IpAddr) -> bool {
        let mut map = self.inner.lock().expect("rate limiter mutex poisoned");
        let now = Instant::now();

        map.retain(|_, (since, _)| now.duration_since(*since) <= self.window);

        let entry = map.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) > self.window {
            entry.0 = now;
            entry.1 = 0;
        }
        entry.1 += 1;
        entry.1 <= self.max_requests
    }
}

impl Clone for RateLimiter {
    fn clone(&self) -> Self {
        RateLimiter {
            inner: Arc::clone(&self.inner),
            max_requests: self.max_requests,
            window: self.window,
        }
    }
}

async fn push_to_homeassistant(
    base_url: &str,
    api_key: &str,
    status: MotionStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    const MAX_RETRIES: u32 = 5;
    const INITIAL_BACKOFF_MS: u64 = 500;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let url = format!(
        "{}/api/states/sensor.pushel_motion",
        base_url.trim_end_matches('/')
    );

    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let body = serde_json::json!({
        "state": status.as_str(),
        "attributes": {
            "friendly_name": "Pushel Motion Detection",
            "last_update": timestamp,
            "device_class": "motion"
        }
    });

    let mut last_error = None;
    for attempt in 0..MAX_RETRIES {
        match client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    info!(
                        "Successfully pushed motion status to Home Assistant: {} (attempt {})",
                        status.as_str(),
                        attempt + 1
                    );
                    return Ok(());
                } else {
                    let status_code = response.status();
                    let _ = response
                        .text()
                        .await;
                    error!(
                        "Failed to push to Home Assistant. Status: {} (attempt {})",
                        status_code,
                        attempt + 1
                    );
                    last_error = Some(format!("HTTP {}", status_code));
                }
            }
            Err(e) => {
                error!(
                    "Network error pushing to Home Assistant: {} (attempt {})",
                    e,
                    attempt + 1
                );
                last_error = Some(format!("Network error: {}", e));
            }
        }

        if attempt < MAX_RETRIES - 1 {
            let backoff_ms = INITIAL_BACKOFF_MS * 2_u64.pow(attempt);
            debug!("Retrying in {}ms...", backoff_ms);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        }
    }

    Err(format!(
        "Failed to push to Home Assistant after {} attempts. Last error: {}",
        MAX_RETRIES,
        last_error.unwrap_or_else(|| "Unknown error".to_string())
    )
    .into())
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn parse_interval(interval: &str) -> Result<u64, String> {
    let len = interval.len();
    if len < 2 {
        return Err("Ungültiges Intervallformat".to_string());
    }

    let (value, unit) = interval.split_at(len - 1);
    let value: u64 = value
        .parse()
        .map_err(|_| "Ungültiger Zahlenwert im Intervall".to_string())?;

    if value == 0 {
        return Err("Intervall muss größer als 0 sein".to_string());
    }

    match unit {
        "s" => Ok(value),
        "m" => Ok(value.checked_mul(60).ok_or("Intervall-Überlauf")?),
        "h" => Ok(value.checked_mul(3600).ok_or("Intervall-Überlauf")?),
        _ => Err("Ungültige Zeiteinheit im Intervall".to_string()),
    }
}

fn send_notification(config: &NotificationConfig) {
    let mut command = Command::new("notify-send");
    command
        .arg(config.title.as_deref().unwrap_or("Erinnerung"))
        .arg(&config.message)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(urgency) = &config.urgency {
        command.arg(format!("--urgency={}", urgency));
    }
    if let Some(expire_time) = config.expire_time {
        command.arg(format!("--expire-time={}", expire_time));
    }
    if let Some(app_name) = &config.app_name {
        command.arg(format!("--app-name={}", app_name));
    }
    if let Some(icon) = &config.icon {
        command.arg(format!("--icon={}", icon));
    }
    if let Some(category) = &config.category {
        command.arg(format!("--category={}", category));
    }
    if config.transient.unwrap_or(false) {
        command.arg("--transient");
    }

    let title = config.title.as_deref().unwrap_or("Erinnerung");

    match command.output() {
        Ok(output) if output.status.success() => {
            info!("Notification sent: {}", title);
        }
        Ok(output) => {
            error!(
                "notify-send failed with exit code: {:?} (notification: {})",
                output.status.code(),
                title
            );
        }
        Err(e) => {
            error!(
                "Error executing notify-send: {} (notification: {})",
                e, title
            );
        }
    }
}

fn create_default_files(config_dir: &PathBuf) -> std::io::Result<()> {
    let default_config = r#"
    {
      "listen_address": "127.0.0.1",
      "port": 3030,
      "webserver_enabled": true,
      "log_format": "pretty",
      "api_token": null,
      "rate_limit_rpm": 60,
      "homeassistant_url": null,
      "homeassistant_api_key": null
    }
    "#;

    let default_notifications = r#"
    [
      {
        "title": "Erinnerung",
        "message": "Trink Wasser!",
        "interval": "30m",
        "urgency": "low",
        "expire_time": 5000,
        "app_name": "Pushel",
        "icon": "dialog-information",
        "category": "reminder",
        "transient": true
      },
      {
        "title": "Erinnerung",
        "message": "Mach mal Pause und strecke dich!",
        "interval": "2h",
        "urgency": "normal",
        "expire_time": 5000,
        "app_name": "Pushel",
        "icon": "dialog-information",
        "category": "reminder",
        "transient": true
      },
      {
        "title": "Erinnerung",
        "message": "Schau in die Ferne, um deine Augen zu entspannen!",
        "interval": "40m",
        "urgency": "low",
        "expire_time": 5000,
        "app_name": "Pushel",
        "icon": "dialog-information",
        "category": "reminder",
        "transient": true
      },
      {
        "title": "Erinnerung",
        "message": "Stehe auf und gehe ein paar Schritte!",
        "interval": "1h",
        "urgency": "normal",
        "expire_time": 5000,
        "app_name": "Pushel",
        "icon": "dialog-information",
        "category": "reminder",
        "transient": true
      },
      {
        "title": "Erinnerung",
        "message": "Überprüfe deine Sitzhaltung!",
        "interval": "15m",
        "urgency": "low",
        "expire_time": 5000,
        "app_name": "Pushel",
        "icon": "dialog-information",
        "category": "reminder",
        "transient": true
      }
    ]
    "#;

    fs::create_dir_all(config_dir)?;

    let config_path = config_dir.join("config.json");
    let mut config_file = File::create(&config_path)?;
    config_file.write_all(default_config.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        config_file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }

    let notifications_path = config_dir.join("notifications.json");
    let mut notifications_file = File::create(&notifications_path)?;
    notifications_file.write_all(default_notifications.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        notifications_file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize X11 for thread-safe operation
    unsafe {
        x11::xlib::XInitThreads();
    }

    let cli = Cli::parse();
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| {
                eprintln!("Fehler: HOME-Umgebungsvariable ist nicht gesetzt");
                std::process::exit(1);
            });
            let mut path = PathBuf::from(home);
            path.push(".config");
            path
        })
        .join("pushel");

    if cli.tui {
        if !config_dir.exists() {
            create_default_files(&config_dir)?;
        }
        let notifications_path = config_dir.join("notifications.json");
        tui::run_tui(notifications_path)?;
        return Ok(());
    }

    if !config_dir.exists() {
        eprintln!(
            "Erstelle Standardkonfigurationsdateien in {:?}...",
            config_dir
        );
        create_default_files(&config_dir)?;
    }

    let config_path = config_dir.join("config.json");
    let notifications_path = config_dir.join("notifications.json");

    let config_data = fs::read_to_string(&config_path)?;
    let app_config: AppConfig = serde_json::from_str(&config_data)?;

    match app_config.log_format.as_str() {
        "json" => tracing_subscriber::fmt().json().init(),
        _ => tracing_subscriber::fmt().pretty().init(),
    }

    info!("Konfigurationsdatei geladen: {:?}", config_path);

    let notifications_data = fs::read_to_string(&notifications_path)?;
    let notifications: Vec<NotificationConfig> = serde_json::from_str(&notifications_data)?;

    info!("Benachrichtigungsdatei geladen: {:?}", notifications_path);

    let motion_tracker = MotionTracker::new(tokio::runtime::Handle::current());

    let motion_tracker_idle = motion_tracker.clone();
    let ha_url = app_config.homeassistant_url.clone();
    let ha_api_key = app_config.homeassistant_api_key.clone();

    thread::spawn(move || {
        info!("Idle detection thread gestartet");
        let mut consecutive_errors = 0u32;
        const MAX_CONSECUTIVE_ERRORS: u32 = 5;
        const ERROR_BACKOFF_SECS: u64 = 30;

        let handle_idle_error = |consecutive_errors: &mut u32, error_msg: &str| {
            *consecutive_errors += 1;
            error!(
                "{} (Fehler {}/{})",
                error_msg, *consecutive_errors, MAX_CONSECUTIVE_ERRORS
            );
            if *consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                error!(
                    "Zu viele aufeinanderfolgende Fehler. Pausiere für {}s...",
                    ERROR_BACKOFF_SECS
                );
                thread::sleep(Duration::from_secs(ERROR_BACKOFF_SECS));
                *consecutive_errors = 0;
            }
        };

        loop {
            let idle_result = std::panic::catch_unwind(UserIdle::get_time);

            match idle_result {
                Ok(Ok(idle)) => {
                    consecutive_errors = 0;
                    let idle_seconds = idle.as_seconds();
                    if idle_seconds < 10 {
                        motion_tracker_idle.update_motion();
                        motion_tracker_idle.update_status(
                            MotionStatus::Active,
                            ha_url.as_deref(),
                            ha_api_key.as_deref(),
                        );
                        debug!("User ist aktiv (idle: {}s)", idle_seconds);
                    } else {
                        motion_tracker_idle.update_status(
                            MotionStatus::Inactive,
                            ha_url.as_deref(),
                            ha_api_key.as_deref(),
                        );
                        debug!("User ist idle ({}s)", idle_seconds);
                    }
                }
                Ok(Err(e)) => {
                    handle_idle_error(
                        &mut consecutive_errors,
                        &format!("Fehler beim Abrufen der Idle-Zeit: {}", e),
                    );
                }
                Err(panic_info) => {
                    handle_idle_error(
                        &mut consecutive_errors,
                        &format!(
                            "PANIC beim Abrufen der Idle-Zeit (vermutlich X11-Fehler): {:?}",
                            panic_info
                        ),
                    );
                }
            }

            thread::sleep(Duration::from_secs(10));
        }
    });

    for notif in notifications {
        let interval = parse_interval(&notif.interval)?;
        let motion_tracker_clone = motion_tracker.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_secs(interval));
            loop {
                if motion_tracker_clone.should_notify() {
                    send_notification(&notif);
                    info!("Motion detected within the last 15 minutes. Sending notification...");
                } else {
                    info!("No motion detected within the last 15 minutes. No notification sent.");
                }
                thread::sleep(Duration::from_secs(interval));
            }
        });
    }

    // Webserver or idle wait
    if app_config.webserver_enabled {
        let rate_limiter = Arc::new(RateLimiter::new(app_config.rate_limit_rpm));
        let api_token = app_config.api_token.clone();
        let rl = rate_limiter.clone();
        let token_check = api_token.clone();

        let push = warp::post()
            .and(warp::path("api"))
            .and(warp::path("v1"))
            .and(warp::path("notify"))
            .and(warp::body::content_length_limit(10 * 1024))
            .and(warp::addr::remote())
            .and(warp::header::optional::<String>("authorization"))
            .and(warp::body::json())
            .map(move |remote: Option<std::net::SocketAddr>, auth_header: Option<String>, notif: AdhocNotification| {
                if let Some(addr) = remote {
                    if !rl.check(addr.ip()) {
                        warn!("Rate limit exceeded for IP: {}", addr.ip());
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": "Rate limit exceeded. Try again later."
                            })),
                            warp::http::StatusCode::TOO_MANY_REQUESTS,
                        );
                    }
                }

                if let Some(ref expected_token) = token_check {
                    let provided = auth_header
                        .as_deref()
                        .and_then(|h| h.strip_prefix("Bearer "))
                        .unwrap_or("");
                    if !constant_time_eq(provided, expected_token.as_str()) {
                        warn!("Unauthorized API request");
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": "Unauthorized. Provide a valid Bearer token."
                            })),
                            warp::http::StatusCode::UNAUTHORIZED,
                        );
                    }
                }

                if notif.message.is_empty() {
                    return warp::reply::with_status(
                        warp::reply::json(&serde_json::json!({
                            "status": "error",
                            "message": "Message cannot be empty"
                        })),
                        warp::http::StatusCode::BAD_REQUEST,
                    );
                }
                if notif.message.len() > MAX_MESSAGE_LENGTH {
                    return warp::reply::with_status(
                        warp::reply::json(&serde_json::json!({
                            "status": "error",
                            "message": format!("Message exceeds maximum length of {MAX_MESSAGE_LENGTH}")
                        })),
                        warp::http::StatusCode::BAD_REQUEST,
                    );
                }
                if let Some(ref urgency) = notif.urgency {
                    if !VALID_URGENCIES.contains(&urgency.as_str()) {
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": format!("Invalid urgency '{}'. Valid values: {:?}", urgency, VALID_URGENCIES)
                            })),
                            warp::http::StatusCode::BAD_REQUEST,
                        );
                    }
                }
                if let Some(ref title) = notif.title {
                    if title.len() > MAX_FIELD_LENGTH {
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": format!("Title exceeds maximum length of {MAX_FIELD_LENGTH}")
                            })),
                            warp::http::StatusCode::BAD_REQUEST,
                        );
                    }
                }
                if let Some(ref app_name) = notif.app_name {
                    if app_name.len() > MAX_FIELD_LENGTH {
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": format!("App name exceeds maximum length of {MAX_FIELD_LENGTH}")
                            })),
                            warp::http::StatusCode::BAD_REQUEST,
                        );
                    }
                }
                if let Some(ref icon) = notif.icon {
                    if icon.contains('/') || icon.contains("..") {
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": "Invalid icon: path separators not allowed"
                            })),
                            warp::http::StatusCode::BAD_REQUEST,
                        );
                    }
                    if icon.len() > MAX_FIELD_LENGTH {
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": format!("Icon exceeds maximum length of {MAX_FIELD_LENGTH}")
                            })),
                            warp::http::StatusCode::BAD_REQUEST,
                        );
                    }
                }
                if let Some(ref category) = notif.category {
                    if category.len() > MAX_FIELD_LENGTH {
                        return warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "status": "error",
                                "message": format!("Category exceeds maximum length of {MAX_FIELD_LENGTH}")
                            })),
                            warp::http::StatusCode::BAD_REQUEST,
                        );
                    }
                }

                send_notification(&NotificationConfig::from(notif));
                warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({
                        "status": "ok",
                        "message": "Notification sent"
                    })),
                    warp::http::StatusCode::OK,
                )
            })
            .with(warp::reply::with::header(
                "X-Content-Type-Options",
                "nosniff",
            ))
            .with(warp::reply::with::header("X-Frame-Options", "DENY"));

        let address = app_config
            .listen_address
            .parse::<std::net::IpAddr>()
            .map_err(|e| {
                format!(
                    "Ungültige listen_address '{}': {}",
                    app_config.listen_address, e
                )
            })?;
        let socket_addr = std::net::SocketAddr::new(address, app_config.port);
        if !address.is_loopback() && app_config.api_token.is_none() {
            warn!(
                "Webserver bound to non-localhost address {} without API token. Consider setting an api_token in config.json for security.",
                address
            );
        }
        info!(
            "Webserver gestartet auf http://{}:{}",
            address, app_config.port
        );

        let (_, server) = warp::serve(push).bind_with_graceful_shutdown(socket_addr, async {
            tokio::signal::ctrl_c().await.ok();
            info!("Signal zum Beenden empfangen, fahre Webserver herunter...");
        });
        server.await;
    } else {
        info!("Webserver deaktiviert. Programm läuft (Ctrl+C zum Beenden)...");
        tokio::signal::ctrl_c().await.ok();
        info!("Signal zum Beenden empfangen.");
    }

    Ok(())
}
