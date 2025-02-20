use serde::{Deserialize, Serialize};
use std::{
    fs,
    fs::File,
    io::Write,
    process::Command,
    thread,
    time::Duration,
    path::PathBuf,
};
use warp::Filter;
use tracing::{info, error};
use tracing_subscriber;

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
      "log_format": "pretty"
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

    // Für jede Benachrichtigung einen eigenen Thread starten
    for notif in notifications {
        let interval = parse_interval(&notif.interval)?;

        thread::spawn(move || {
            // Warte das angegebene Intervall vor der ersten Benachrichtigung
            thread::sleep(Duration::from_secs(interval));
            loop {
                send_notification(&notif);
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
