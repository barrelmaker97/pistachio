use super::*;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusRecorder};
use std::future::Future;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{
    TcpListener,
    tcp::{OwnedReadHalf, OwnedWriteHalf},
};

async fn with_async_recorder<F: Future>(recorder: &PrometheusRecorder, f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    std::future::poll_fn(|cx| metrics::with_local_recorder(recorder, || f.as_mut().poll(cx))).await
}

struct FakeNutConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl FakeNutConn {
    async fn accept(listener: &TcpListener) -> Self {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, write_half) = stream.into_split();
        let mut conn = Self { reader: BufReader::new(read_half), writer: write_half };
        assert_eq!(conn.read_line().await, "NETVER");
        conn.write_all("1.3\n").await;
        conn
    }

    async fn read_line(&mut self) -> String {
        let mut line = String::new();
        self.reader.read_line(&mut line).await.unwrap();
        line.trim_end_matches('\n').to_string()
    }

    async fn write_all(&mut self, data: &str) {
        self.writer.write_all(data.as_bytes()).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn handle_list_vars(&mut self, ups: &str, vars: &[(&str, &str)]) {
        assert_eq!(self.read_line().await, format!("LIST VAR {ups}"));
        let mut response = format!("BEGIN LIST VAR {ups}\n");
        for (name, val) in vars {
            response.push_str(&format!("VAR {ups} {name} \"{val}\"\n"));
        }
        response.push_str(&format!("END LIST VAR {ups}\n"));
        self.write_all(&response).await;
    }

    async fn handle_get_ups_vars(&mut self, ups: &str, vars: &[(&str, &str, &str)]) {
        let var_pairs: Vec<(&str, &str)> = vars.iter().map(|(name, val, _)| (*name, *val)).collect();
        self.handle_list_vars(ups, &var_pairs).await;
        for (name, _, desc) in vars {
            assert_eq!(self.read_line().await, format!("GET DESC {ups} {name}"));
            self.write_all(&format!("DESC {ups} {name} \"{desc}\"\n")).await;
        }
    }

    async fn handle_logout(&mut self) {
        assert_eq!(self.read_line().await, "LOGOUT");
        self.write_all("OK Goodbye\n").await;
    }
}

#[test]
fn parse_default_args() {
    let args = Args::try_parse_from(["pistachio"]).unwrap();
    assert_eq!(args.ups_name, DEFAULT_UPS_NAME);
    assert_eq!(args.ups_host, DEFAULT_UPS_HOST);
    assert_eq!(args.ups_port, DEFAULT_UPS_PORT);
    assert_eq!(args.bind_ip, DEFAULT_BIND_IP);
    assert_eq!(args.bind_port, DEFAULT_BIND_PORT);
    assert_eq!(args.poll_rate, DEFAULT_POLL_RATE);
}

#[test]
fn parse_override_args() {
    let args = Args::try_parse_from([
        "pistachio",
        "--ups-name",
        "test_ups",
        "--ups-host",
        "192.168.1.1",
        "--ups-port",
        "1234",
        "--bind-ip",
        "10.0.0.1",
        "--bind-port",
        "5678",
        "--poll-rate",
        "5",
    ])
    .unwrap();
    assert_eq!(args.ups_name, "test_ups");
    assert_eq!(args.ups_host, "192.168.1.1");
    assert_eq!(args.ups_port, 1234);
    assert_eq!(args.bind_ip, IpAddr::V4("10.0.0.1".parse().unwrap()));
    assert_eq!(args.bind_port, 5678);
    assert_eq!(args.poll_rate, 5);
}

#[test]
fn parse_poll_rate_zero_fails() {
    let result = Args::try_parse_from(["pistachio", "--poll-rate", "0"]);
    assert!(result.is_err());
}

#[test]
fn parse_invalid_bind_ip_fails() {
    let result = Args::try_parse_from(["pistachio", "--bind-ip", "not-an-ip"]);
    assert!(result.is_err());
}

#[test]
fn parse_invalid_ups_port_fails() {
    let result = Args::try_parse_from(["pistachio", "--ups-port", "70000"]);
    assert!(result.is_err());
}

#[test]
fn build_metrics_basic() {
    let mut ups_vars = HashMap::new();
    ups_vars.insert("input.voltage".to_string(), (String::from("122.0"), String::from("Nominal input voltage")));
    ups_vars.insert("ups.load".to_string(), (String::from("25.5"), String::from("UPS load in percent")));
    ups_vars.insert("battery.charge".to_string(), (String::from("100.0"), String::from("Battery charge in percent")));
    let expected_metric_name1 = convert_var_name("input.voltage");
    let expected_metric_name2 = convert_var_name("ups.load");
    let expected_metric_name3 = convert_var_name("battery.charge");

    let metrics = Metrics::build(&ups_vars);

    assert_eq!(metrics.basic_gauges.len(), 3);
    assert_eq!(*metrics.basic_gauges.get("input.voltage").unwrap(), expected_metric_name1);
    assert_eq!(*metrics.basic_gauges.get("ups.load").unwrap(), expected_metric_name2);
    assert_eq!(*metrics.basic_gauges.get("battery.charge").unwrap(), expected_metric_name3);
}

#[test]
fn build_metrics_not_float() {
    let mut ups_vars = HashMap::new();
    let var_name = "ups.mfr";
    ups_vars.insert(var_name.to_string(), (String::from("CPS"), String::from("UPS Manufacturer")));

    let metrics = Metrics::build(&ups_vars);

    assert_eq!(metrics.basic_gauges.len(), 0);
}

#[test]
fn build_metrics_label_gauges() {
    let ups_vars = HashMap::new();

    let metrics = Metrics::build(&ups_vars);

    assert_eq!(metrics.count(), 2);
    assert_eq!(metrics.label_gauges.len(), 2);
}

#[test]
fn convert_var_does_not_have_ups_prefix() {
    let var_name = "input.voltage";
    let expected_metric_name = "ups_input_voltage";

    let metric_name = convert_var_name(var_name);

    assert_eq!(metric_name, expected_metric_name);
}

#[test]
fn convert_var_already_has_ups_prefix() {
    let var_name = "ups.load";
    let expected_metric_name = "ups_load";

    let metric_name = convert_var_name(var_name);

    assert_eq!(metric_name, expected_metric_name);
}

#[test]
fn convert_var_multiple_dots() {
    let var_name = "input.voltage.nominal";
    let expected_metric_name = "ups_input_voltage_nominal";

    let metric_name = convert_var_name(var_name);

    assert_eq!(metric_name, expected_metric_name);
}

#[test]
fn convert_var_empty() {
    let var_name = "";
    let expected_metric_name = "ups_";

    let metric_name = convert_var_name(var_name);

    assert_eq!(metric_name, expected_metric_name);
}

#[test]
fn update_basic_and_label_metrics() {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();

    let mut ups_vars = HashMap::new();
    ups_vars.insert("input.voltage".to_string(), ("120.0".to_string(), "Input voltage".to_string()));
    ups_vars.insert("ups.load".to_string(), ("15.0".to_string(), "UPS load".to_string()));

    let var_list = vec![
        rups::Variable::parse("input.voltage", "122.5".to_string()),
        rups::Variable::parse("ups.load", "30.0".to_string()),
        rups::Variable::parse("ups.status", "OB DISCHRG".to_string()),
        rups::Variable::parse("ups.beeper.status", "muted".to_string()),
        rups::Variable::parse("ups.mfr", "CPS".to_string()),
    ];

    metrics::with_local_recorder(&recorder, || {
        let metrics = Metrics::build(&ups_vars);
        metrics.update(&var_list);
    });

    let rendered = handle.render();
    assert!(rendered.contains("# HELP ups_input_voltage Input voltage"));
    assert!(rendered.contains("# TYPE ups_input_voltage gauge"));
    assert!(rendered.contains("ups_input_voltage 122.5"));
    assert!(rendered.contains("ups_load 30"));
    assert!(rendered.contains("ups_status{status=\"OB\"} 1"));
    assert!(rendered.contains("ups_status{status=\"DISCHRG\"} 1"));
    assert!(rendered.contains("ups_status{status=\"CHRG\"} 0"));
    assert!(rendered.contains("ups_status{status=\"OL\"} 0"));
    assert!(rendered.contains("ups_beeper_status{status=\"muted\"} 1"));
    assert!(rendered.contains("ups_beeper_status{status=\"enabled\"} 0"));
    assert!(rendered.contains("ups_beeper_status{status=\"disabled\"} 0"));
}

#[test]
fn update_basic_gauge_ignores_invalid_float() {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();

    let mut ups_vars = HashMap::new();
    ups_vars.insert("input.voltage".to_string(), ("120.0".to_string(), "Input voltage".to_string()));

    metrics::with_local_recorder(&recorder, || {
        let metrics = Metrics::build(&ups_vars);
        metrics.update(&vec![rups::Variable::parse("input.voltage", "121.0".to_string())]);
        metrics.update(&vec![rups::Variable::parse("input.voltage", "not-a-float".to_string())]);
    });

    let rendered = handle.render();
    assert!(rendered.contains("ups_input_voltage 121"));
}

#[test]
fn reset_metrics_sets_all_gauges_to_zero() {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();

    let mut ups_vars = HashMap::new();
    ups_vars.insert("input.voltage".to_string(), ("120.0".to_string(), "Input voltage".to_string()));

    metrics::with_local_recorder(&recorder, || {
        let metrics = Metrics::build(&ups_vars);
        metrics.update(&vec![
            rups::Variable::parse("input.voltage", "122.0".to_string()),
            rups::Variable::parse("ups.status", "OL CHRG".to_string()),
            rups::Variable::parse("ups.beeper.status", "enabled".to_string()),
        ]);
        metrics.reset();
    });

    let rendered = handle.render();
    assert!(rendered.contains("ups_input_voltage 0"));
    for state in UPS_STATES {
        assert!(rendered.contains(&format!("ups_status{{status=\"{state}\"}} 0")));
    }
    for state in BEEPER_STATES {
        assert!(rendered.contains(&format!("ups_beeper_status{{status=\"{state}\"}} 0")));
    }
}

#[tokio::test]
async fn create_connection_and_get_ups_vars_success() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let mut conn = FakeNutConn::accept(&listener).await;
        conn.handle_get_ups_vars(
            "ups",
            &[
                ("input.voltage", "122.0", "Input voltage"),
                ("ups.load", "25.5", "UPS load"),
                ("ups.status", "OL", "UPS status"),
            ],
        )
        .await;
        conn.handle_logout().await;
    });

    let args = Args::try_parse_from(["pistachio", "--ups-port", &port.to_string()]).unwrap();
    let mut conn = create_connection(&args).await.unwrap();
    let ups_vars = get_ups_vars(&args, &mut conn).await.unwrap();
    conn.close().await.unwrap();
    server.await.unwrap();

    assert_eq!(ups_vars.len(), 3);
    assert_eq!(ups_vars.get("input.voltage"), Some(&("122.0".to_string(), "Input voltage".to_string())));
    assert_eq!(ups_vars.get("ups.load"), Some(&("25.5".to_string(), "UPS load".to_string())));
    assert_eq!(ups_vars.get("ups.status"), Some(&("OL".to_string(), "UPS status".to_string())));
}

#[tokio::test]
async fn create_connection_fails_when_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let args = Args::try_parse_from(["pistachio", "--ups-port", &port.to_string()]).unwrap();
    let result = create_connection(&args).await;
    assert!(result.is_err());
}

#[tokio::test(start_paused = true)]
async fn monitor_ups_polls_recovers_from_io_error_and_shuts_down() {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let server_handle = handle.clone();
    let server = tokio::spawn(async move {
        // Initial connection + variable discovery
        let mut conn1 = FakeNutConn::accept(&listener).await;
        conn1.handle_get_ups_vars("ups", &[("input.voltage", "120.0", "Input voltage")]).await;

        // Poll 1: healthy update
        conn1.handle_list_vars("ups", &[("input.voltage", "121.5"), ("ups.status", "OL")]).await;

        // Poll 2: wait for request, verify Poll 1 metrics were recorded, then drop socket to cause I/O error
        assert_eq!(conn1.read_line().await, "LIST VAR ups");
        let rendered_after_poll1 = server_handle.render();
        assert!(rendered_after_poll1.contains("ups_input_voltage 121.5"));
        assert!(rendered_after_poll1.contains("ups_status{status=\"OL\"} 1"));
        drop(conn1);

        // monitor_ups resets metrics to 0 and reconnects on ClientError::Io
        let mut conn2 = FakeNutConn::accept(&listener).await;
        let rendered_after_reset = server_handle.render();
        assert!(rendered_after_reset.contains("ups_input_voltage 0"));
        assert!(rendered_after_reset.contains("ups_status{status=\"OL\"} 0"));

        // Poll 3: recovered connection updates metrics
        conn2.handle_list_vars("ups", &[("input.voltage", "123.0"), ("ups.status", "OB DISCHRG")]).await;

        // Poll 4: wait for request to confirm Poll 3 processed, then trigger graceful shutdown
        assert_eq!(conn2.read_line().await, "LIST VAR ups");
        let rendered_after_recovery = server_handle.render();
        assert!(rendered_after_recovery.contains("ups_input_voltage 123"));
        assert!(rendered_after_recovery.contains("ups_status{status=\"OB\"} 1"));
        assert!(rendered_after_recovery.contains("ups_status{status=\"DISCHRG\"} 1"));
        assert!(rendered_after_recovery.contains("ups_status{status=\"CHRG\"} 0"));

        conn2.write_all("BEGIN LIST VAR ups\nVAR ups input.voltage \"123.0\"\nEND LIST VAR ups\n").await;
        shutdown_tx.send(()).unwrap();
        conn2.handle_logout().await;
    });

    let args = Args::try_parse_from(["pistachio", "--ups-port", &port.to_string(), "--poll-rate", "5"]).unwrap();
    with_async_recorder(&recorder, async {
        let mut conn = create_connection(&args).await.unwrap();
        let ups_vars = get_ups_vars(&args, &mut conn).await.unwrap();
        let metrics = Metrics::build(&ups_vars);
        monitor_ups(conn, args, metrics, shutdown_rx).await;
    })
    .await;

    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn monitor_ups_handles_reconnect_and_shutdown_close_errors() {
    let recorder = PrometheusBuilder::new().build_recorder();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let server = tokio::spawn(async move {
        let mut conn = FakeNutConn::accept(&listener).await;
        conn.handle_get_ups_vars("ups", &[("input.voltage", "120.0", "Input voltage")]).await;

        // Wait for Poll 1 request, then drop both the connection and the listener so reconnect fails
        assert_eq!(conn.read_line().await, "LIST VAR ups");
        drop(conn);
        drop(listener);

        // Advance past two poll_rate ticks (11s) so monitor_ups writes to the closed socket,
        // hits ClientError::Io, and fails to reconnect because the listener was dropped.
        time::sleep(Duration::from_secs(11)).await;
        shutdown_tx.send(()).unwrap();
    });

    let args = Args::try_parse_from(["pistachio", "--ups-port", &port.to_string(), "--poll-rate", "5"]).unwrap();
    with_async_recorder(&recorder, async {
        let mut conn = create_connection(&args).await.unwrap();
        let ups_vars = get_ups_vars(&args, &mut conn).await.unwrap();
        let metrics = Metrics::build(&ups_vars);
        monitor_ups(conn, args, metrics, shutdown_rx).await;
    })
    .await;

    server.await.unwrap();
}
