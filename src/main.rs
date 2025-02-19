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
    interval: String, // Zeit als String mit Suffix
}

#[derive(Debug, Deserialize, Serialize)]
struct AdhocNotification {
    title: Option<String>,
    message: String,
}

#[derive(Debug, Deserialize)]
struct AppConfig {
    listen_address: String,
    port: u16,
    webserver_enabled: bool,
    default_title: String,
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

fn send_notification(title: &str, message: &str) {
    if let Err(e) = Command::new("notify-send")
        .arg(title)
        .arg(message)
        .status()
    {
        error!("Fehler beim Senden der Benachrichtigung: {}", e);
    } else {
        info!("Benachrichtigung gesendet: {} - {}", title, message);
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
        "interval": "1h"
      },
      {
        "title": "Erinnerung",
        "message": "Mach mal Pause und strecke dich!",
        "interval": "2h"
      },
      {
        "title": "Erinnerung",
        "message": "Schau in die Ferne, um deine Augen zu entspannen!",
        "interval": "30m"
      },
      {
        "title": "Erinnerung",
        "message": "Stehe auf und gehe ein paar Schritte!",
        "interval": "1h"
      },
      {
        "title": "Erinnerung",
        "message": "Überprüfe deine Sitzhaltung!",
        "interval": "45m"
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
        let title = notif.title.clone().unwrap_or_else(|| app_config.default_title.clone());
        let interval = parse_interval(&notif.interval)?;

        thread::spawn(move || {
            // Warte das angegebene Intervall vor der ersten Benachrichtigung
            thread::sleep(Duration::from_secs(interval));
            loop {
                send_notification(&title, &notif.message);
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
                let title = notif.title.unwrap_or_else(|| app_config.default_title.clone());
                send_notification(&title, &notif.message);
                warp::reply::json(&"Notification sent")
            });

        let address = app_config.listen_address.parse::<std::net::IpAddr>()?;
        let socket_addr = std::net::SocketAddr::new(address, app_config.port);
        info!("Webserver gestartet auf http://{}:{}", address, app_config.port);
        warp::serve(push).run(socket_addr).await;
    }

    Ok(())
}
