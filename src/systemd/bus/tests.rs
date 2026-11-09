use super::*;
use std::{
    io::Write,
    os::{
        fd::{AsFd, OwnedFd},
        unix::net::UnixStream,
    },
    sync::{Arc, Mutex},
};
use zbus::{blocking::connection::Builder, zvariant::Fd};

#[derive(Default)]
struct Calls {
    starts: usize,
    stops: usize,
    kills: usize,
    releases: usize,
    owner: String,
    environment: Vec<String>,
}
struct FakeManager(Arc<Mutex<Calls>>);
#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl FakeManager {
    #[zbus(property)]
    fn environment(&self) -> Vec<String> {
        self.0.lock().unwrap().environment.clone()
    }
    #[zbus(property)]
    fn version(&self) -> &str {
        "257.13"
    }
    fn start_transient_unit(
        &self,
        name: &str,
        mode: &str,
        properties: Vec<(String, OwnedValue)>,
        auxiliary: Vec<(String, Vec<(String, OwnedValue)>)>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        assert_eq!(name, "synthetic.service");
        assert_eq!(mode, "fail");
        assert!(auxiliary.is_empty());
        assert_eq!(properties.len(), 1);
        let value = properties.into_iter().next().unwrap().1;
        let fd: Fd<'_> = value.try_into().unwrap();
        let fd: OwnedFd = fd.try_into().unwrap();
        let mut file = std::fs::File::from(fd);
        file.write_all(b"synthetic output").unwrap();
        self.0.lock().unwrap().starts += 1;
        Ok(OwnedObjectPath::try_from("/job/1").unwrap())
    }
    fn get_unit(&self, name: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        if name == "missing" {
            return Err(zbus::fdo::Error::UnknownObject("synthetic".into()));
        }
        Ok(OwnedObjectPath::try_from("/unit/1").unwrap())
    }
    fn stop_unit(&self, name: &str, mode: &str) -> OwnedObjectPath {
        assert_eq!(name, "synthetic.service");
        assert_eq!(mode, "replace");
        self.0.lock().unwrap().stops += 1;
        OwnedObjectPath::try_from("/job/2").unwrap()
    }
    fn kill_unit(&self, name: &str, who: &str, signal: i32) {
        assert_eq!(name, "synthetic.service");
        assert_eq!(who, "all");
        assert_eq!(signal, 9);
        self.0.lock().unwrap().kills += 1;
    }
    fn unref_unit(&self, name: &str) {
        assert_eq!(name, "synthetic.service");
        self.0.lock().unwrap().releases += 1;
    }
}
struct FakeBus(Arc<Mutex<Calls>>);
#[zbus::interface(name = "org.freedesktop.DBus")]
impl FakeBus {
    fn get_name_owner(&self, name: &str) -> String {
        assert_eq!(name, DESTINATION);
        let owner = self.0.lock().unwrap().owner.clone();
        if owner.is_empty() {
            ":1.7".into()
        } else {
            owner
        }
    }
    fn get_connection_unix_user(&self, owner: &str) -> u32 {
        assert_eq!(owner, ":1.7");
        1000
    }
}
struct FakeUnit;
#[zbus::interface(name = "org.freedesktop.systemd1.Unit")]
impl FakeUnit {
    #[zbus(property)]
    fn description(&self) -> &str {
        "synthetic"
    }
    #[zbus(property)]
    fn transient(&self) -> bool {
        true
    }
    #[zbus(property, name = "InvocationID")]
    fn invocation_id(&self) -> Vec<u8> {
        vec![1; 16]
    }
    #[zbus(property)]
    fn active_state(&self) -> &str {
        "active"
    }
    #[zbus(property)]
    fn sub_state(&self) -> &str {
        "running"
    }
    #[zbus(property)]
    fn job(&self) -> (u32, OwnedObjectPath) {
        (0, OwnedObjectPath::try_from("/").unwrap())
    }
}
struct FakeService;
#[zbus::interface(name = "org.freedesktop.systemd1.Service")]
impl FakeService {
    #[zbus(property, name = "Type")]
    fn service_type(&self) -> &str {
        "exec"
    }
    #[zbus(property)]
    fn control_group(&self) -> &str {
        "/synthetic"
    }
    #[zbus(property, name = "MainPID")]
    fn main_pid(&self) -> u32 {
        42
    }
    #[zbus(property)]
    fn exec_main_exit_timestamp_monotonic(&self) -> u64 {
        0
    }
    #[zbus(property)]
    fn exec_main_code(&self) -> i32 {
        0
    }
    #[zbus(property)]
    fn exec_main_status(&self) -> i32 {
        0
    }
}

#[test]
fn exchanges_real_dbus_messages_with_a_private_test_peer() {
    let (server, client) = UnixStream::pair().unwrap();
    let calls = Arc::new(Mutex::new(Calls::default()));
    let state = calls.clone();
    let thread = std::thread::spawn(move || {
        Builder::async_io_unix_stream(server)
            .server(zbus::Guid::generate())
            .unwrap()
            .p2p()
            .serve_at("/org/freedesktop/DBus", FakeBus(state.clone()))
            .unwrap()
            .serve_at(PATH, FakeManager(state))
            .unwrap()
            .serve_at("/unit/1", FakeUnit)
            .unwrap()
            .serve_at("/unit/1", FakeService)
            .unwrap()
            .build()
            .unwrap()
    });
    let connection = Builder::async_io_unix_stream(client)
        .p2p()
        .method_timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let _server = thread.join().unwrap();
    let mut bus = UserBus {
        connection,
        owner: None,
    };
    assert_eq!(bus.version(), Ok("257.13".into()));
    let file = tempfile::tempfile().unwrap();
    bus.start(
        "synthetic.service",
        &[(
            "StandardOutputFileDescriptor",
            Value::from(Fd::from(file.as_fd())),
        )],
    )
    .unwrap();
    let raw = bus.snapshot("synthetic.service").unwrap();
    assert_eq!(raw.main_pid, 42);
    assert_eq!(raw.invocation, vec![1; 16]);
    assert_eq!(raw.control_group, "/synthetic");
    assert!(raw.transient);
    assert_eq!(raw.service_type, "exec");
    assert_eq!(raw.job, 0);
    assert_eq!(bus.snapshot("missing"), Err(ManagerError::Unavailable));
    bus.stop("synthetic.service", StopMode::Graceful).unwrap();
    bus.stop("synthetic.service", StopMode::Force).unwrap();
    bus.release("synthetic.service").unwrap();
    assert_eq!(bus.authenticated_uid(), Ok(1000));
    assert_eq!(bus.authenticated_uid(), Ok(1000));
    assert_eq!(bus.owner.as_deref(), Some(":1.7"));
    calls.lock().unwrap().environment =
        vec!["SYNTHETIC=private".into(), "LD_PRELOAD=synthetic".into()];
    assert_eq!(
        bus.environment_names(),
        Ok(vec!["SYNTHETIC".into(), "LD_PRELOAD".into()])
    );
    calls.lock().unwrap().environment = vec!["invalid".into()];
    assert_eq!(bus.environment_names(), Err(ManagerError::Unavailable));
    calls.lock().unwrap().environment = vec!["A=x".into(); 4097];
    assert_eq!(bus.environment_names(), Err(ManagerError::Unavailable));
    calls.lock().unwrap().owner = ":1.8".into();
    assert_eq!(bus.authenticated_uid(), Err(ManagerError::Unavailable));
    let calls = calls.lock().unwrap();
    assert_eq!(
        (calls.starts, calls.stops, calls.kills, calls.releases),
        (1, 1, 1, 1)
    );
    let mut file = file;
    use std::io::{Read, Seek};
    file.rewind().unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "synthetic output");
}

#[test]
fn classifies_errors_without_exposing_details_and_rejects_bad_properties() {
    for (name, expected) in [
        ("org.freedesktop.systemd1.NoSuchUnit", ManagerError::Missing),
        (
            "org.freedesktop.systemd1.UnitExists",
            ManagerError::Conflict,
        ),
        (
            "org.freedesktop.DBus.Error.AccessDenied",
            ManagerError::Rejected,
        ),
        (
            "org.freedesktop.DBus.Error.InvalidArgs",
            ManagerError::Rejected,
        ),
        (
            "org.freedesktop.DBus.Error.NoReply",
            ManagerError::Unavailable,
        ),
    ] {
        let message = zbus::message::Message::signal("/", "org.example.Test", "Synthetic")
            .unwrap()
            .build(&())
            .unwrap();
        let error = zbus::Error::MethodError(
            name.try_into().unwrap(),
            Some("synthetic-private-data".into()),
            message,
        );
        assert_eq!(classify(error), expected);
    }
    assert_eq!(
        classify(zbus::Error::Failure("synthetic".into())),
        ManagerError::Unavailable
    );
    let mut props = HashMap::new();
    assert_eq!(
        get::<u32>(&mut props, "missing"),
        Err(ManagerError::Unavailable)
    );
    props.insert("wrong".into(), OwnedValue::from(true));
    assert_eq!(
        get::<u32>(&mut props, "wrong"),
        Err(ManagerError::Unavailable)
    );
}
