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
fn importer_recursively_rejects_match_exec_without_executing_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let included = dir.path().join("included");
    let marker = dir.path().join("executed");
    fs::write(
        &config,
        format!(
            "Include {}\nHost vm\n HostName 192.0.2.1\n User user\n",
            included.display()
        ),
    )
    .unwrap();
    fs::write(
        included,
        format!("Match exec \"touch {}\"\n", marker.display()),
    )
    .unwrap();
    assert_eq!(
        importer::preview(&config, &["vm".into()]).unwrap_err().code,
        "dynamic_config_requires_manual_import"
    );
    assert!(!marker.exists());
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
    assert_eq!(jump.connections[0].authentication, Authentication::Agent);
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
