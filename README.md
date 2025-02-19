# Pushel

<img src="./pushel.webp" style="width:300px"/>

Pushel is a simple reminder application that sends desktop notifications at specified intervals. It also includes a web server to handle API requests for sending ad-hoc notifications.

## Features

- Send desktop notifications at specified intervals.
- Configurable via JSON files.
- Web server to handle API requests for ad-hoc notifications.
- Configurable logging format (pretty or JSON).

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
  "listen_address": "127.0.0.1",
  "port": 3030,
  "webserver_enabled": true,
  "default_title": "Erinnerung",
  "log_format": "pretty"
}
```

### Example `notifications.json`

```json
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
```

## API Usage

Pushel includes a web server that listens for API requests to send ad-hoc notifications. The server is enabled by setting `webserver_enabled` to `true` in `config.json`.

### Example API Request

To send an ad-hoc notification, send a POST request to `http://127.0.0.1:3030/api/v1/notify` with the following JSON payload:

```json
{
  "title": "Optionaler Titel",
  "message": "Die Nachricht, die angezeigt werden soll"
}
```

### Example `curl` Command

```sh
curl -X POST http://127.0.0.1:3030/api/v1/notify \
     -H "Content-Type: application/json" \
     -d '{
           "title": "Optionaler Titel",
           "message": "Die Nachricht, die angezeigt werden soll"
         }'
```

## Logging

Pushel supports two logging formats: `pretty` and `json`. The logging format can be configured in `config.json` using the `log_format` field.

### Example `config.json` with JSON Logging

```json
{
  "listen_address": "127.0.0.1",
  "port": 3030,
  "webserver_enabled": true,
  "default_title": "Erinnerung",
  "log_format": "json"
}
```

## License

This project is licensed under the MIT License.
