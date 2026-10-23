use kuma_client::monitor::HttpBodyEncoding;

#[test]
fn accepts_uppercase_from_new_kuma_versions() {
    let v: HttpBodyEncoding = serde_json::from_str("\"JSON\"").unwrap();
    assert_eq!(v, HttpBodyEncoding::Json);
    let v: HttpBodyEncoding = serde_json::from_str("\"FORM\"").unwrap();
    assert_eq!(v, HttpBodyEncoding::Form);
    let v: HttpBodyEncoding = serde_json::from_str("\"XML\"").unwrap();
    assert_eq!(v, HttpBodyEncoding::Xml);
}

#[test]
fn still_accepts_lowercase() {
    let v: HttpBodyEncoding = serde_json::from_str("\"json\"").unwrap();
    assert_eq!(v, HttpBodyEncoding::Json);
}

#[test]
fn serializes_to_lowercase() {
    assert_eq!(
        serde_json::to_string(&HttpBodyEncoding::Json).unwrap(),
        "\"json\""
    );
}

#[test]
fn deserializes_smtp_monitor_type() {
    let payload = serde_json::json!({
        "1": {
            "id": 1,
            "name": "SMTP Example",
            "type": "smtp",
            "hostname": "mail.example.com",
            "port": 587,
            "interval": 60,
            "active": true
        }
    });
    let list: std::collections::HashMap<String, kuma_client::monitor::Monitor> =
        serde_json::from_value(payload).unwrap();
    let m = list.get("1").unwrap();
    assert_eq!(m.common().id().clone(), Some(1));
    assert_eq!(m.common().name().clone(), Some("SMTP Example".to_owned()));
    assert!(matches!(m, kuma_client::monitor::Monitor::Smtp { .. }));
    assert_eq!(m.monitor_type(), kuma_client::monitor::MonitorType::Smtp);
}

#[test]
fn tolerates_fractional_numbers() {
    use kuma_client::monitor::MonitorList;
    // interval 为小数（Kuma 2.0.x 某些字段允许小数），原版会报 "Unable to represent 3.5 as i32"
    let payload = serde_json::json!({
        "1": {
            "id": 1,
            "name": "Fractional",
            "type": "group",
            "interval": 3.5
        }
    });
    let list: MonitorList = serde_json::from_value(payload).unwrap();
    let m = list.get("1").unwrap();
    assert_eq!(m.common().id().clone(), Some(1));
    assert_eq!(m.common().interval().clone(), Some(4)); // 3.5 四舍五入为 4
}
