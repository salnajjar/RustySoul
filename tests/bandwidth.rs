// Run the exact vendored limiter's tests as part of the application's ordinary acceptance suite.
#[path = "../vendor/soulseek-rs-lib/src/utils/bandwidth.rs"]
mod bandwidth;

#[test]
fn legacy_total_only_setting_uses_the_same_budget() {
    let limits = std::sync::Arc::new(bandwidth::Bandwidth::default());
    limits.set_total(1000);
    assert_eq!(
        limits
            .for_user("alice")
            .take(10000, || false)
            .unwrap()
            .bytes,
        100
    );
}

#[test]
fn payload_counts_actual_io_and_refunds_not_reservations() {
    let limits = std::sync::Arc::new(bandwidth::Bandwidth::default());
    let user = limits.for_user("alice");
    drop(user.take(1000, || false).unwrap());
    assert_eq!(limits.payload_bytes(), 0);
    user.take(100, || false).unwrap().finish(25);
    let other = limits.for_user("bob");
    other.take(300, || false).unwrap().finish(200);
    assert_eq!(limits.payload_bytes(), 225);
    assert_eq!(
        limits.take_payload_users(),
        std::collections::HashMap::from([("alice".into(), 25), ("bob".into(), 200)])
    );
    assert!(limits.take_payload_users().is_empty());
    limits.configure(0, 0, vec![]);
    assert_eq!(limits.payload_bytes(), 225);
}
