use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn cancellation_fixture() -> (
    MihomoRuntime<crate::mihomo::TcpControllerTransport>,
    Arc<AtomicBool>,
) {
    let address = "127.0.0.1:1".parse().unwrap();
    let transport =
        crate::mihomo::TcpControllerTransport::new(address, "test", Duration::from_secs(1))
            .unwrap();
    let mut runtime = MihomoRuntime::new(
        MihomoClient::new(transport),
        address,
        "test",
        RealtimeOptions::default(),
    )
    .unwrap();
    let topics = [
        RealtimeTopic::Traffic,
        RealtimeTopic::Memory,
        RealtimeTopic::Connections,
        RealtimeTopic::Logs,
    ];
    let cancelled: Arc<Vec<_>> = Arc::new(
        topics
            .iter()
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect(),
    );
    let serial_wait = Arc::new(AtomicBool::new(false));
    for (index, topic) in topics.into_iter().enumerate() {
        let own_cancel = cancelled[index].clone();
        let group = cancelled.clone();
        let failed = serial_wait.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !group[index].load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            // A join must not prevent the remaining workers from receiving cancellation.
            let deadline = Instant::now() + Duration::from_secs(1);
            while !group.iter().all(|flag| flag.load(Ordering::Acquire)) {
                if Instant::now() >= deadline {
                    failed.store(true, Ordering::Release);
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        runtime.subscriptions.insert(
            topic,
            RealtimeSubscription::for_shutdown_test(own_cancel, worker),
        );
    }
    (runtime, serial_wait)
}

#[test]
fn shutdown_cancels_all_subscriptions_before_joining_any_worker() {
    let (mut runtime, serial_wait) = cancellation_fixture();
    runtime.stop_realtime();
    assert!(runtime.subscriptions.is_empty());
    assert!(
        !serial_wait.load(Ordering::Acquire),
        "cancellation was serialized behind a join"
    );
    runtime.stop_realtime();
}

#[test]
fn shutdown_drop_also_cancels_all_subscriptions_before_joining() {
    let (runtime, serial_wait) = cancellation_fixture();
    drop(runtime);
    assert!(
        !serial_wait.load(Ordering::Acquire),
        "Drop serialized cancellation behind a join"
    );
}
