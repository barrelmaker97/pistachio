use super::*;

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
