# pistachio
Prometheus Exporter for Network UPS Tools

## Configuration
All config is done via environment variables, listed below:
| Parameter     | Description                                                                                      | Default     |
|---------------|--------------------------------------------------------------------------------------------------|-------------|
| `UPS_NAME`    | Name of the UPS to monitor                                                                       | `ups`       |
| `UPS_HOST`    | Hostname of the NUT server to monitor                                                            | `localhost` |
| `UPS_PORT`    | Port of the NUT server to monitor                                                                | `3493`      |
| `RUST_LOG`    | Logging level of the exporter                                                                    | `info`      |
| `POLL_RATE`   | Amount of time, in seconds, this exporter will wait between requests to the NUT server for data  | `10`        |

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
