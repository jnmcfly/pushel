use serde::{Deserialize, Serialize};
use std::{
    fs,
    fs::File,
    io::Write,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    path::PathBuf,
};
use warp::Filter;
use tracing::{info, error, debug};
use tracing_subscriber;
use user_idle::UserIdle;

#[derive(Debug, Deserialize)]
struct NotificationConfig {
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

#[derive(Debug, Deserialize)]
struct AppConfig {
    listen_address: String,
    port: u16,
    webserver_enabled: bool,
    log_format: String,
    homeassistant_url: Option<String>,
    homeassistant_api_key: Option<String>,
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
                return Instant::now().duration_since(last_motion_time) <= Duration::from_secs(15 * 60);
            }
        }
        false
    }

    fn update_status(&self, new_status: MotionStatus, ha_url: Option<&str>, ha_api_key: Option<&str>) {
        if let Ok(mut current_status) = self.current_status.lock() {
            if *current_status != new_status {
                info!("Motion status changed: {:?} -> {:?}", *current_status, new_status);
                *current_status = new_status;

                // Push to Home Assistant if configured
                if let (Some(url), Some(api_key)) = (ha_url, ha_api_key) {
                    let url = url.to_string();
                    let api_key = api_key.to_string();
                    self.runtime_handle.spawn(async move {
                        if let Err(e) = push_to_homeassistant(&url, &api_key, new_status).await {
                            error!("Failed to push status to Home Assistant: {}", e);
                        }
                    });
                }
            }
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

async fn push_to_homeassistant(base_url: &str, api_key: &str, status: MotionStatus) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/states/sensor.pushel_motion", base_url.trim_end_matches('/'));

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs();

    let body = serde_json::json!({
        "state": status.as_str(),
        "attributes": {
            "friendly_name": "Pushel Motion Detection",
            "last_update": timestamp,
            "device_class": "motion"
        }
    });

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if response.status().is_success() {
        info!("Successfully pushed motion status to Home Assistant: {}", status.as_str());
    } else {
        error!("Failed to push to Home Assistant. Status: {}, Response: {:?}", response.status(), response.text().await?);
    }

    Ok(())
}

fn parse_interval(interval: &str) -> Result<u64, &'static str> {
    let len = interval.len();
    if len < 2 {
        return Err("Ungültiges Intervallformat");
    }

    let (value, unit) = interval.split_at(len - 1);
    let value: u64 = value.parse().map_err(|_| "Ungültiger Zahlenwert im Intervall")?;

    match unit {
        "s" => Ok(value),
        "m" => Ok(value * 60),
        "h" => Ok(value * 3600),
        _ => Err("Ungültige Zeiteinheit im Intervall"),
    }
}

fn send_notification(config: &NotificationConfig) {
    let mut command = Command::new("notify-send");
    command.arg(config.title.as_deref().unwrap_or("Erinnerung"))
           .arg(&config.message);

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

    if let Err(e) = command.status() {
        error!("Fehler beim Senden der Benachrichtigung: {}", e);
    } else {
        info!("Benachrichtigung gesendet: {} - {}", config.title.as_deref().unwrap_or("Erinnerung"), config.message);
    }
}

fn create_default_files(config_dir: &PathBuf) -> std::io::Result<()> {
    let default_config = r#"
    {
      "listen_address": "0.0.0.0",
      "port": 3030,
      "webserver_enabled": true,
      "default_title": "Erinnerung",
      "log_format": "pretty",
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
    let mut config_file = File::create(config_path)?;
    config_file.write_all(default_config.as_bytes())?;

    let notifications_path = config_dir.join("notifications.json");
    let mut notifications_file = File::create(notifications_path)?;
    notifications_file.write_all(default_notifications.as_bytes())?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize X11 for thread-safe operation
    unsafe {
        x11::xlib::XInitThreads();
    }

    // Bestimme den Pfad zur Konfigurationsdatei im Standard-Linux-Konfigurationsverzeichnis
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let mut path = PathBuf::from(std::env::var("HOME").unwrap());
            path.push(".config");
            path
        }).join("pushel");

    // Prüfe, ob der Konfigurationsordner existiert, und erstelle ihn bei Bedarf
    if !config_dir.exists() {
        info!("Erstelle Standardkonfigurationsdateien...");
        create_default_files(&config_dir)?;
    }

    let config_path = config_dir.join("config.json");
    let notifications_path = config_dir.join("notifications.json");

    // Lese die Konfigurationsdatei ein
    let config_data = fs::read_to_string(&config_path)?;
    let app_config: AppConfig = serde_json::from_str(&config_data)?;

    // Initialisiere Tracing Subscriber basierend auf der Konfiguration
    match app_config.log_format.as_str() {
        "json" => tracing_subscriber::fmt().json().init(),
        _ => tracing_subscriber::fmt().pretty().init(),
    }

    info!("Konfigurationsdatei geladen: {:?}", config_path);

    // Lese die Benachrichtigungsdatei ein
    let notifications_data = fs::read_to_string(&notifications_path)?;
    let notifications: Vec<NotificationConfig> = serde_json::from_str(&notifications_data)?;

    info!("Benachrichtigungsdatei geladen: {:?}", notifications_path);

    let motion_tracker = MotionTracker::new(tokio::runtime::Handle::current());

    // Starte idle detection thread
    let motion_tracker_idle = motion_tracker.clone();
    let ha_url = app_config.homeassistant_url.clone();
    let ha_api_key = app_config.homeassistant_api_key.clone();

    thread::spawn(move || {
        info!("Idle detection thread gestartet");
        loop {
            match UserIdle::get_time() {
                Ok(idle) => {
                    let idle_seconds = idle.as_seconds();
                    // Wenn User aktiv ist (idle < 10 Sekunden), update motion
                    if idle_seconds < 10 {
                        motion_tracker_idle.update_motion();
                        motion_tracker_idle.update_status(
                            MotionStatus::Active,
                            ha_url.as_deref(),
                            ha_api_key.as_deref()
                        );
                        debug!("User ist aktiv (idle: {}s)", idle_seconds);
                    } else {
                        motion_tracker_idle.update_status(
                            MotionStatus::Inactive,
                            ha_url.as_deref(),
                            ha_api_key.as_deref()
                        );
                        info!("User ist idle ({}s)", idle_seconds);
                    }
                }
                Err(e) => {
                    error!("Fehler beim Abrufen der Idle-Zeit: {}", e);
                }
            }
            // Prüfe alle 10 Sekunden
            thread::sleep(Duration::from_secs(10));
        }
    });

    for notif in notifications {
        let interval = parse_interval(&notif.interval)?;
        let motion_tracker_clone = motion_tracker.clone();

        thread::spawn(move || {
            // Warte das angegebene Intervall vor der ersten Benachrichtigung
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

    // Webserver für Adhoc-Benachrichtigungen
    if app_config.webserver_enabled {
        let push = warp::post()
            .and(warp::path("api"))
            .and(warp::path("v1"))
            .and(warp::path("notify"))
            .and(warp::body::json())
            .map(move |notif: AdhocNotification| {
                let config = NotificationConfig {
                    title: notif.title,
                    message: notif.message,
                    interval: String::new(), // Not used for ad-hoc notifications
                    urgency: notif.urgency,
                    expire_time: notif.expire_time,
                    app_name: notif.app_name,
                    icon: notif.icon,
                    category: notif.category,
                    transient: notif.transient,
                };
                send_notification(&config);
                warp::reply::json(&"Notification sent")
            });

        let address = app_config.listen_address.parse::<std::net::IpAddr>()?;
        let socket_addr = std::net::SocketAddr::new(address, app_config.port);
        info!("Webserver gestartet auf http://{}:{}", address, app_config.port);
        warp::serve(push).run(socket_addr).await;
    }

    Ok(())
}
