use super::*;
use metrics_exporter_prometheus::PrometheusBuilder;

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
