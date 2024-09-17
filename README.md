# Pistachio
**Pistachio** is a Prometheus exporter written in Rust, designed for monitoring UPS devices using Network UPS Tools (NUT).

## Features

- **Efficient Monitoring**: Written in Rust for high performance and minimal resource consumption.
- **Prometheus Integration**: Exposes UPS metrics directly to Prometheus for easy monitoring and alerting.
- **Configurable**: All settings can be customized via environment variables to suit your deployment needs.
- **Resilient**: Graceful error handling to ensure consistent operation.

## Configuration

Pistachio can be configured using either command-line options or by setting corresponding environment variables.
Command-line options take precedence over environment variables.
Below is a breakdown of the available options:

| Option                    | Description                                                                     | Environment Variable | Default     |
|---------------------------|---------------------------------------------------------------------------------|----------------------|-------------|
| `--ups-name <UPS_NAME>`   | Name of the UPS to monitor.                                                     | `UPS_NAME`           | `ups`       |
| `--ups-host <UPS_HOST>`   | Hostname of the NUT server to monitor.                                          | `UPS_HOST`           | `127.0.0.1` |
| `--ups-port <UPS_PORT>`   | Port of the NUT server to monitor.                                              | `UPS_PORT`           | `3493`      |
| `--bind-ip <BIND_IP>`     | IP address on which the exporter will serve metrics.                            | `BIND_IP`            | `0.0.0.0`   |
| `--bind-port <BIND_PORT>` | Port on which the exporter will serve metrics.                                  | `BIND_PORT`          | `9120`      |
| `--poll-rate <POLL_RATE>` | Time in seconds between requests to the NUT server. Must be at least 2 seconds. | `POLL_RATE`          | `10`        |
| `-h, --help`              | Print help message                                                              | -                    | -           |
| `-V, --version`           | Print version information                                                       | -                    | -           |

### Example

To run Pistachio with custom values for `UPS_HOST` and `POLL_RATE`, you can either use the command-line options:

```bash
pistachio --ups-host 192.168.1.100 --poll-rate 5
```

Or set the environment variables:

```bash
export UPS_HOST=192.168.1.100
export POLL_RATE=5
pistachio
```

## Docker Image

A pre-built Docker image of Pistachio is available on the GitHub Container Registry.
This allows you to easily deploy the exporter in any containerized environment.

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
  -e RUST_LOG=info \
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
      RUST_LOG: info
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

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
