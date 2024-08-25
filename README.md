# Pistachio
**Pistachio** is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

## Configuration
Configuration is managed through environment variables, detailed below:
| Parameter     | Description                                                                                      | Default     |
|---------------|--------------------------------------------------------------------------------------------------|-------------|
| `UPS_NAME`    | Name of the UPS to monitor                                                                       | `ups`       |
| `UPS_HOST`    | Hostname of the NUT server to monitor                                                            | `localhost` |
| `UPS_PORT`    | Port of the NUT server to monitor                                                                | `3493`      |
| `BIND_PORT`   | Port on which the exporter will serve metrics for Prometheus                                     | `9120`      |
| `RUST_LOG`    | Logging level of the exporter                                                                    | `info`      |
| `POLL_RATE`   | Amount of time, in seconds, this exporter will wait between requests to the NUT server for data  | `10`        |

## Features

- **Efficient Monitoring**: Written in Rust for high performance and minimal resource consumption.
- **Prometheus Integration**: Exposes UPS metrics directly to Prometheus for easy monitoring and alerting.
- **Configurable**: All settings can be customized via environment variables to suit your deployment needs.
- **Resilient**: Graceful error handling to ensure consistent operation.

## Docker Image

A pre-built Docker image of Pistachio is available on the GitHub Container Registry. This allows you to easily deploy the exporter in any containerized environment.

### Pulling the Image

You can pull the Docker image from the GitHub Container Registry with the following command:

```bash
docker pull ghcr.io/barrelmaker97/pistachio:latest
```

### Running the Exporter with Docker

To run the exporter using Docker, use the following command:

```bash
docker run -d \
  --name pistachio \
  -p 9120:9120 \
  -e UPS_NAME=your_ups_name \
  -e UPS_HOST=your_nut_server_host \
  -e UPS_PORT=3493 \
  -e LOG_LEVEL=info \
  -e POLL_RATE=10 \
  ghcr.io/barrelmaker97/pistachio:latest
```

Replace the environment variables (`UPS_NAME`, `UPS_HOST`, etc.) with the appropriate values for your setup.

### Example Docker Compose Configuration

Here’s an example of how you can include Pistachio in a Docker Compose setup:

```yaml
version: '3.7'

services:
  pistachio:
    image: ghcr.io/barrelmaker97/pistachio:latest
    environment:
      UPS_NAME: your_ups_name
      UPS_HOST: your_nut_server_host
      UPS_PORT: 3493
      LOG_LEVEL: info
      POLL_RATE: 10
    ports:
      - "9120:9120"
```

## Getting Started

1. Clone the repository:
    ```bash
    git clone https://github.com/barrelmaker97/pistachio.git
    ```

2. Navigate to the project directory:
    ```bash
    cd pistachio
    ```

3. Build the project using Cargo:
    ```bash
    cargo build --release
    ```

4. Run the exporter:
    ```bash
    ./target/release/pistachio
    ```

5. Configure Prometheus to scrape metrics from the exporter at `http://<your_host>:<BIND_PORT>/metrics`.

# License

Copyright (c) 2024 Nolan Cooper

This exporter is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This exporter is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this exporter.  If not, see <https://www.gnu.org/licenses/>.
