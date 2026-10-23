use ssh_manager_core::*;
use std::{
    fs,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[test]
fn identity_survives_rename_and_export_drops_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut h = Host::new(
        "vm".into(),
        "192.0.2.1".into(),
        "user".into(),
        22,
        Authentication::Password {
            credential_ref: Some("private-reference".into()),
        },
    );
    h.context.description = "保留上下文".into();
    store.save(&h).unwrap();
    h.name = "renamed".into();
    h.connections[0].address = "192.0.2.2".into();
    store.save(&h).unwrap();
    let read = store.host("renamed").unwrap();
    assert_eq!(read.id, h.id);
    assert_eq!(read.context.description, "保留上下文");
    let json = serde_json::to_string(&store.export().unwrap()).unwrap();
    assert!(!json.contains("private-reference"));
}
#[test]
fn catalog_import_is_atomic_and_never_overwrites_existing_context() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let h = Host::new(
        "vm".into(),
        "192.0.2.1".into(),
        "user".into(),
        22,
        Authentication::Agent,
    );
    store.save(&h).unwrap();
    let other = Host::new(
        "other".into(),
        "192.0.2.2".into(),
        "user".into(),
        22,
        Authentication::Agent,
    );
    assert!(
        store
            .import_catalog(CatalogExport {
                schema_version: 1,
                hosts: vec![other, h]
            })
            .is_err()
    );
    assert_eq!(store.list().unwrap().len(), 1);
}
#[test]
fn removing_jump_in_use_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let jump = Host::new(
        "jump".into(),
        "192.0.2.1".into(),
        "user".into(),
        22,
        Authentication::Agent,
    );
    store.save(&jump).unwrap();
    let mut target = Host::new(
        "target".into(),
        "192.0.2.2".into(),
        "user".into(),
        22,
        Authentication::Agent,
    );
    target.connections[0].jumps.push(jump.default_connection);
    store.save(&target).unwrap();
    assert_eq!(store.remove("jump").unwrap_err().code, "host_in_use");
}
#[test]
fn command_and_config_injection_are_rejected_at_the_model_boundary() {
    let mut h = Host::new(
        "bad".into(),
        "127.0.0.1\nProxyCommand touch /tmp/no".into(),
        "user".into(),
        22,
        Authentication::Agent,
    );
    assert!(h.validate().is_err());
    h.connections[0].address = "::1".into();
    h.connections[0].user = "user;touch /tmp/no".into();
    assert!(h.validate().is_err());
    assert_eq!(ssh::shell_quote("/tmp/a'b"), "'/tmp/a'\\''b'");
}
#[test]
fn importer_skips_match_blocks_without_executing_them() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let included = dir.path().join("included");
    let marker = dir.path().join("executed");
    fs::write(
        &config,
        format!(
            "Include \"{}\"\nHost vm\n HostName 192.0.2.1\n User user\nMatch originalhost web\n ProxyCommand unsafe-command\nHost web\n HostName 192.0.2.2\n User user\nMatch all\n Port 2222\n",
            included.display()
        ),
    )
    .unwrap();
    // The block opened at the end of the included file continues into the outer file until "Host vm".
    fs::write(
        included,
        format!(
            "Match exec \"touch '{}'\"\n HostName 198.51.100.1\n",
            marker.display()
        ),
    )
    .unwrap();
    let plan = importer::preview(&config, &["vm".into(), "web".into()]).unwrap();
    assert!(!marker.exists());
    assert_eq!(plan.skipped.len(), 2);
    assert!(plan.skipped[0].contains("HostName"));
    let vm = &plan.hosts[0].connections[0];
    assert_eq!(vm.address, "192.0.2.1");
    assert_eq!(vm.port, 2222);
    assert!(vm.issues.is_empty());
    let web = &plan.hosts[1].connections[0];
    assert_eq!(web.port, 2222);
    assert_eq!(web.issues.len(), 1);
    assert!(web.issues[0].contains("Match"));
}
#[test]
fn importer_ignores_client_options_and_maps_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let key = dir.path().join("key");
    let socket = dir.path().join("agent dir/agent.sock");
    fs::write(&key, "").unwrap();
    fs::write(
        &config,
        format!(
            "Host *\n IgnoreUnknown UseKeychain\n UseKeychain yes\n AddKeysToAgent yes\nHost pw\n HostName 192.0.2.1\n User user\n PubkeyAuthentication no\nHost op\n HostName 192.0.2.2\n User user\n IdentityAgent \"{}\"\n IdentityFile \"{}\"\n IdentitiesOnly yes\nHost kbd\n HostName 192.0.2.3\n User user\n PreferredAuthentications keyboard-interactive\n",
            socket.display(),
            key.display()
        ),
    )
    .unwrap();
    let plan = importer::preview(&config, &[]).unwrap();
    assert_eq!(plan.ignored.len(), 3);
    let pw = &plan.hosts[0].connections[0];
    assert!(matches!(pw.authentication, Authentication::Password { .. }));
    assert!(pw.issues.is_empty());
    let op = &plan.hosts[1].connections[0];
    assert!(matches!(op.authentication, Authentication::Key { .. }));
    assert_eq!(op.identity_agent.as_deref(), Some(socket.to_str().unwrap()));
    assert!(op.issues.is_empty());
    assert_eq!(plan.hosts[2].connections[0].issues.len(), 1);
}
#[test]
fn static_import_preserves_defaults_multivalue_keys_and_jump_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let key1 = dir.path().join("one");
    let key2 = dir.path().join("two");
    fs::write(&key1, "").unwrap();
    fs::write(&key2, "").unwrap();
    fs::write(&config,format!("Host *\n User user\n Port 2222\nHost jump\n HostName 192.0.2.1\nHost web blog\n HostName 192.0.2.2\n IdentityFile {}\n IdentityFile {}\n ProxyJump jump\nHost other\n HostName 192.0.2.3\n ProxyCommand unsafe-command\n",key1.display(),key2.display())).unwrap();
    let plan = importer::preview(&config, &["web".into()]).unwrap();
    assert_eq!(plan.hosts.len(), 2);
    let web = plan.hosts.iter().find(|h| h.name == "web").unwrap();
    let jump = plan.hosts.iter().find(|h| h.name == "jump").unwrap();
    assert_eq!(web.connections[0].port, 2222);
    assert_eq!(web.connections[0].jumps, vec![jump.default_connection]);
    assert!(web.connections[0].issues.is_empty());
    // Naming no key, OpenSSH ends up asking for the password.
    assert!(matches!(
        jump.connections[0].authentication,
        Authentication::Password { .. }
    ));
    assert!(jump.connections[0].issues.is_empty());
    match &web.connections[0].authentication {
        Authentication::Key { paths, .. } => assert_eq!(paths.len(), 2),
        _ => panic!("explicit keys missing"),
    }
}
#[test]
fn output_capture_drains_after_limit_and_detects_completion() {
    let mut cmd = std::process::Command::new("sh");
    cmd.args(["-c","printf '\\036test:start\\037' >&2; head -c 100000 /dev/zero; printf '\\036test:exit:255\\037' >&2; exit 255"]);
    let result = process::run(
        cmd,
        None,
        Duration::from_secs(2),
        Arc::new(AtomicBool::new(false)),
        32,
        Some("test".into()),
    )
    .unwrap();
    assert_eq!(result.stdout.total, 100000);
    assert_eq!(result.stdout.bytes.len(), 32);
    assert_eq!(result.stderr.remote_exit, Some(255));
    assert!(result.stderr.started);
}
#[test]
fn timeouts_kill_local_descendants_and_release_pipes() {
    let mut cmd = std::process::Command::new("sh");
    cmd.args(["-c", "sleep 10 & wait"]);
    let start = std::time::Instant::now();
    let result = process::run(
        cmd,
        None,
        Duration::from_millis(100),
        Arc::new(AtomicBool::new(false)),
        32,
        None,
    )
    .unwrap();
    assert_eq!(result.termination.as_deref(), Some("timeout"));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn explicit_missing_identity_is_reported_without_discarding_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let present = dir.path().join("present");
    let missing = dir.path().join("missing");
    fs::write(&present, "").unwrap();
    fs::write(&config,format!("Host vm\n HostName 192.0.2.1\n User user\n IdentitiesOnly yes\n IdentityFile {}\n IdentityFile {}\n",present.display(),missing.display())).unwrap();
    let p = importer::preview(&config, &["vm".into()]).unwrap();
    let c = &p.hosts[0].connections[0];
    assert!(c.identities_only);
    assert_eq!(c.issues.len(), 1);
    match &c.authentication {
        Authentication::Key { paths, .. } => assert_eq!(paths.len(), 2),
        _ => panic!("missing key authentication"),
    }
}

#[test]
fn first_use_catalog_creation_is_serialized_across_callers() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::sync::Arc::new(dir.path().to_path_buf());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let root = root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Store::open(&*root).unwrap().list().unwrap()
            })
        })
        .collect();
    for thread in threads {
        assert!(thread.join().unwrap().is_empty());
    }
}
#[test]
fn import_carries_over_known_hosts_and_edits_forget_the_old_server() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let known = dir.path().join("known hosts");
    let key = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    fs::write(
        &known,
        format!("@cert-authority *.test ssh-ed25519 {key}\n[web.test]:2222 ssh-ed25519 {key}\n"),
    )
    .unwrap();
    fs::write(
        &config,
        format!(
            "Host web\n HostName web.test\n Port 2222\n User user\n UserKnownHostsFile \"{}\"\nHost new\n HostName new.test\n User user\n UserKnownHostsFile \"{}\"\n",
            known.display(),
            known.display()
        ),
    )
    .unwrap();
    let store = Store::open(dir.path().join("data")).unwrap();
    importer::apply(&store, importer::preview(&config, &[]).unwrap()).unwrap();
    let web = store.host("web").unwrap();
    let keys = trust::list(&store, web.default_connection).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key, key);
    let new = store.host("new").unwrap();
    assert!(
        trust::list(&store, new.default_connection)
            .unwrap()
            .is_empty()
    );

    struct Keychain(std::cell::RefCell<Vec<String>>);
    impl CredentialStore for Keychain {
        fn get(&self, _: &str) -> Result<zeroize::Zeroizing<String>> {
            unreachable!()
        }
        fn set(&self, _: &str, _: &str) -> Result<()> {
            unreachable!()
        }
        fn delete(&self, reference: &str) -> Result<()> {
            self.0.borrow_mut().push(reference.into());
            Ok(())
        }
    }
    let keychain = Keychain(Default::default());
    let mut edited = web.clone();
    edited.connections[0].authentication = Authentication::Password {
        credential_ref: Some("saved".into()),
    };
    credentials::update_host(&store, &keychain, &edited).unwrap();
    assert_eq!(
        trust::list(&store, web.default_connection).unwrap().len(),
        1
    );
    edited.name = "renamed".into();
    edited.connections[0].port = 22;
    edited.connections[0].authentication = Authentication::Agent;
    credentials::update_host(&store, &keychain, &edited).unwrap();
    assert!(
        trust::list(&store, web.default_connection)
            .unwrap()
            .is_empty()
    );
    assert_eq!(*keychain.0.borrow(), vec!["saved".to_string()]);
}
#[test]
fn only_keys_with_a_passphrase_ask_for_one() {
    let dir = tempfile::tempdir().unwrap();
    for (name, passphrase) in [("plain", ""), ("locked", "secret phrase")] {
        let status = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
            .arg(dir.path().join(name))
            .status()
            .unwrap();
        assert!(status.success());
    }
    let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    assert!(!credentials::key_has_passphrase(&path("plain")));
    assert!(credentials::key_has_passphrase(&path("locked")));
    assert!(!credentials::key_has_passphrase(&path("missing")));
}
#[test]
fn hosts_naming_no_key_import_as_password_unless_keys_are_required() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    fs::write(
        &config,
        "Host plain\n HostName 192.0.2.1\nHost keys-only\n HostName 192.0.2.2\n PasswordAuthentication no\nHost agent\n HostName 192.0.2.3\n IdentityAgent /tmp/agent.sock\n",
    )
    .unwrap();
    let plan = importer::preview(&config, &[]).unwrap();
    let auth = |name: &str| {
        plan.hosts
            .iter()
            .find(|h| h.name == name)
            .unwrap()
            .connections[0]
            .authentication
            .clone()
    };
    assert!(matches!(auth("plain"), Authentication::Password { .. }));
    assert!(!matches!(
        auth("keys-only"),
        Authentication::Password { .. }
    ));
    // Default keys that exist on this machine are kept next to the agent.
    assert!(!matches!(auth("agent"), Authentication::Password { .. }));
}
#[test]
fn reachability_probe_wants_an_ssh_server() {
    use std::{io::Write, net::TcpListener};
    let serve = |greeting: &'static [u8]| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(greeting).unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });
        port
    };
    let timeout = Duration::from_secs(2);
    assert!(reach::probe(
        "127.0.0.1",
        serve(b"SSH-2.0-OpenSSH_9.9\r\n"),
        timeout
    ));
    assert!(!reach::probe(
        "127.0.0.1",
        serve(b"HTTP/1.1 400 Bad Request\r\n"),
        timeout
    ));
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert!(!reach::probe("127.0.0.1", closed, timeout));
}
