use super::*;
use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering};

fn reply_hello(stream: &mut UnixStream) -> RequestFrame {
    let hello = super::super::protocol::read_request(stream).unwrap();
    super::super::protocol::write_response(
        stream,
        &ResponseFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: hello.session_id.clone(),
            request_id: hello.request_id,
            body: Ok(Response::Hello { helper_pid: 99 }),
        },
    )
    .unwrap();
    hello
}

fn reply_control_hello(stream: &mut UnixStream) {
    let hello = super::super::protocol::read_control_request(stream).unwrap();
    super::super::protocol::write_control_response(
        stream,
        &ControlResponseFrame {
            protocol_version: PROTOCOL_VERSION,
            session_id: hello.session_id,
            request_id: hello.request_id,
            body: Ok(ControlResponse::Hello),
        },
    )
    .unwrap();
}

#[test]
fn broken_helper_is_not_retried_and_next_explicit_call_restarts_cleanly() {
    let connections = Arc::new(AtomicUsize::new(0));
    let connector_count = connections.clone();
    let connector: Arc<Connector> = Arc::new(move || {
        let attempt = connector_count.fetch_add(1, Ordering::SeqCst);
        let (client, mut server) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let _hello = reply_hello(&mut server);
            let request = super::super::protocol::read_request(&mut server).unwrap();
            if attempt == 0 {
                return;
            }
            super::super::protocol::write_response(
                &mut server,
                &ResponseFrame {
                    protocol_version: PROTOCOL_VERSION,
                    session_id: request.session_id,
                    request_id: request.request_id,
                    body: Ok(Response::Permissions(PermissionStatus {
                        accessibility: true,
                        screen_recording: true,
                        screen_recording_restart_required: false,
                    })),
                },
            )
            .unwrap();
        });
        Ok(RawConnection {
            stream: client,
            control_stream: None,
            managed_service: false,
            socket_path: None,
        })
    });
    let backend = MacHelperBackend::with_connector(connector);

    assert!(matches!(
        backend.permissions(),
        Err(ComputerUseError::HelperUnavailable(_))
    ));
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    assert!(backend.permissions().unwrap().accessibility);
    assert_eq!(connections.load(Ordering::SeqCst), 2);
}

#[test]
fn replayed_or_cross_session_response_fails_closed() {
    let connector: Arc<Connector> = Arc::new(move || {
        let (client, mut server) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let _hello = reply_hello(&mut server);
            let request = super::super::protocol::read_request(&mut server).unwrap();
            super::super::protocol::write_response(
                &mut server,
                &ResponseFrame {
                    protocol_version: PROTOCOL_VERSION,
                    session_id: "attacker-session".to_string(),
                    request_id: request.request_id - 1,
                    body: Ok(Response::Permissions(PermissionStatus::default())),
                },
            )
            .unwrap();
        });
        Ok(RawConnection {
            stream: client,
            control_stream: None,
            managed_service: false,
            socket_path: None,
        })
    });
    let backend = MacHelperBackend::with_connector(connector);
    assert!(matches!(
        backend.permissions(),
        Err(ComputerUseError::HelperProtocol(_))
    ));
}

#[test]
fn stalled_helper_hits_the_deadline_without_an_automatic_retry() {
    let connections = Arc::new(AtomicUsize::new(0));
    let connector_count = connections.clone();
    let connector: Arc<Connector> = Arc::new(move || {
        connector_count.fetch_add(1, Ordering::SeqCst);
        let (client, mut server) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let _hello = reply_hello(&mut server);
            let _request = super::super::protocol::read_request(&mut server).unwrap();
            std::thread::sleep(Duration::from_secs(1));
        });
        Ok(RawConnection {
            stream: client,
            control_stream: None,
            managed_service: false,
            socket_path: None,
        })
    });
    let backend = MacHelperBackend::with_connector(connector);

    let error = backend.permissions().unwrap_err();
    assert!(matches!(error, ComputerUseError::HelperUnavailable(_)));
    assert!(error.to_string().contains("deadline"));
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}

#[test]
fn cancellation_channel_is_not_blocked_by_a_stalled_primary_request() {
    let (request_started_tx, request_started_rx) = std::sync::mpsc::channel();
    let connector: Arc<Connector> = Arc::new(move || {
        let (client, mut server) = UnixStream::pair().unwrap();
        let (control_client, mut control_server) = UnixStream::pair().unwrap();
        let request_started_tx = request_started_tx.clone();
        std::thread::spawn(move || {
            let _hello = reply_hello(&mut server);
            let _request = super::super::protocol::read_request(&mut server).unwrap();
            request_started_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(1));
        });
        std::thread::spawn(move || {
            reply_control_hello(&mut control_server);
            let request =
                super::super::protocol::read_control_request(&mut control_server).unwrap();
            super::super::protocol::write_control_response(
                &mut control_server,
                &ControlResponseFrame {
                    protocol_version: PROTOCOL_VERSION,
                    session_id: request.session_id,
                    request_id: request.request_id,
                    body: Ok(ControlResponse::CancelAck(CancelAck {
                        lease_id: Some("lease-test".to_string()),
                        quiesced: true,
                        helper_terminated: false,
                    })),
                },
            )
            .unwrap();
        });
        Ok(RawConnection {
            stream: client,
            control_stream: Some(control_client),
            managed_service: false,
            socket_path: None,
        })
    });
    let backend = Arc::new(MacHelperBackend::with_connector(connector));
    backend
        .manager
        .lock()
        .unwrap()
        .connection()
        .expect("establish test connection");
    let request_backend = backend.clone();
    let stalled = std::thread::spawn(move || request_backend.permissions());
    request_started_rx
        .recv_timeout(Duration::from_millis(100))
        .unwrap();

    let started = std::time::Instant::now();
    let ack = backend.cancel_active().unwrap();
    assert!(ack.quiesced);
    assert_eq!(ack.lease_id.as_deref(), Some("lease-test"));
    assert!(started.elapsed() < Duration::from_millis(150));
    assert!(stalled.join().unwrap().is_err());
}

#[test]
fn release_service_path_cannot_escape_the_resources_directory() {
    let resources_directory = tempfile::tempdir().unwrap();
    let service = resources_directory.path().join(SERVICE_APP_NAME);
    let executable = service
        .join("Contents")
        .join("MacOS")
        .join(SERVICE_EXECUTABLE);
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    File::create(&executable).unwrap();
    let expected_directory = resources_directory.path().canonicalize().unwrap();
    assert_eq!(
        validate_service_app_path(service, Some(&expected_directory)).unwrap(),
        expected_directory.join(SERVICE_APP_NAME)
    );

    let outside = tempfile::tempdir().unwrap();
    let outside_service = outside.path().join(SERVICE_APP_NAME);
    let outside_executable = outside_service
        .join("Contents")
        .join("MacOS")
        .join(SERVICE_EXECUTABLE);
    std::fs::create_dir_all(outside_executable.parent().unwrap()).unwrap();
    File::create(outside_executable).unwrap();
    assert!(matches!(
        validate_service_app_path(outside_service, Some(&expected_directory)),
        Err(ComputerUseError::HelperRejected(_))
    ));
}
