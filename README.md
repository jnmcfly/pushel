# Pushel

<img src="./pushel.webp" style="width:300px"/>

Pushel is a simple reminder application that sends desktop notifications at specified intervals. It also includes a web server to handle API requests for sending ad-hoc notifications.

## Features

- Send desktop notifications at specified intervals.
- Configurable via JSON files.
- Web server to handle API requests for ad-hoc notifications.
- Configurable logging format (pretty or JSON).
- Support for additional notification options: urgency, expire-time, app-name, icon, category, and transient.
- Motion detection to prevent notifications when user is away.
- Home Assistant integration to report user activity status.

## Installation

1. Clone the repository:
    ```sh
    git clone https://github.com/yourusername/pushel.git
    cd pushel
    ```

2. Build the project:
    ```sh
    cargo build --release
    ```

3. Run the application:
    ```sh
    ./target/release/pushel
    ```

## Configuration

Pushel uses two configuration files located in the standard Linux configuration directory (`$XDG_CONFIG_HOME/pushel` or `$HOME/.config/pushel`):

1. `config.json`: Contains the application configuration.
2. `notifications.json`: Contains the notifications to be sent.

### Example `config.json`

```json
{
  "listen_address": "0.0.0.0",
  "port": 3030,
  "webserver_enabled": true,
  "default_title": "Erinnerung",
  "log_format": "pretty",
  "homeassistant_url": null,
  "homeassistant_api_key": null
}
```

**Note**: Set `homeassistant_url` and `homeassistant_api_key` to `null` if you don't want to use Home Assistant integration.

### Example `notifications.json`

```json
[
  {
    "title": "Erinnerung",
    "message": "Trink Wasser!",
    "interval": "1h",
    "urgency": "low",
    "expire_time": 5000,
    "app_name": "Pushel",
    "icon": "dialog-information",
    "category": "reminder",
    "transient": true
  }
]
```

## API Usage

Pushel includes a web server that listens for API requests to send ad-hoc notifications. The server is enabled by setting `webserver_enabled` to `true` in `config.json`.

### Example API Request

To send an ad-hoc notification, send a POST request to `http://127.0.0.1:3030/api/v1/notify` with the following JSON payload:

```json
{
  "title": "Optionaler Titel",
  "message": "Die Nachricht, die angezeigt werden soll",
  "urgency": "normal",
  "expire_time": 5000,
  "app_name": "Pushel",
  "icon": "dialog-information",
  "category": "reminder",
  "transient": true
}
```

### Example `curl` Command

```sh
curl -X POST http://127.0.0.1:3030/api/v1/notify \
     -H "Content-Type: application/json" \
     -d '{
           "title": "Optionaler Titel",
           "message": "Die Nachricht, die angezeigt werden soll",
           "urgency": "normal",
           "expire_time": 5000,
           "app_name": "Pushel",
           "icon": "dialog-information",
           "category": "reminder",
           "transient": true
         }'
```

## Logging

Pushel supports two logging formats: `pretty` and `json`. The logging format can be configured in `config.json` using the `log_format` field.

### Example `config.json` with JSON Logging

```json
{
  "listen_address": "0.0.0.0",
  "port": 3030,
  "webserver_enabled": true,
  "default_title": "Erinnerung",
  "log_format": "json"
}
```

## Home Assistant Integration

Pushel can integrate with Home Assistant to report user activity status. When configured, Pushel will automatically create and update a motion sensor in Home Assistant that tracks whether the user is active or inactive at their computer.

### Configuration

To enable Home Assistant integration, add the following fields to your `config.json`:

```json
{
  "listen_address": "0.0.0.0",
  "port": 3030,
  "webserver_enabled": true,
  "default_title": "Erinnerung",
  "log_format": "pretty",
  "homeassistant_url": "http://your-homeassistant-instance:8123",
  "homeassistant_api_key": "your-long-lived-access-token"
}
```

### Parameters

- **homeassistant_url**: The base URL of your Home Assistant instance (e.g., `http://192.168.1.100:8123` or `https://homeassistant.local`)
- **homeassistant_api_key**: A long-lived access token from Home Assistant. You can create one in Home Assistant under Profile → Security → Long-Lived Access Tokens.

### Home Assistant Sensor

Once configured, Pushel will create and update a sensor entity in Home Assistant:

- **Entity ID**: `sensor.pushel_motion`
- **Friendly Name**: Pushel Motion Detection
- **Device Class**: motion
- **States**:
  - `active`: User is actively using the computer (idle time < 10 seconds)
  - `inactive`: User is idle (idle time ≥ 10 seconds)

The sensor includes an attribute `last_update` with a timestamp of the last status change.

### Usage in Home Assistant

You can use this sensor in your Home Assistant automations, scripts, or dashboards. For example:

#### Example Automation

```yaml
automation:
  - alias: "Pause notifications when user is away"
    trigger:
      - platform: state
        entity_id: sensor.pushel_motion
        to: 'inactive'
        for: '00:05:00'
    action:
      - service: notify.mobile_app
        data:
          message: "User has been away for 5 minutes"
```

#### Example Dashboard Card

```yaml
type: entity
entity: sensor.pushel_motion
name: Computer Activity
```

### Troubleshooting

- Ensure your Home Assistant instance is accessible from the machine running Pushel
- Verify your long-lived access token is valid and has not expired
- Check Pushel logs for any connection errors to Home Assistant
- The motion status updates occur every 10 seconds based on user idle time

## License

This project is licensed under the MIT License.
