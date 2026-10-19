//! cargo run -p ssh-manager-core --example check -- /absolute/path/to/ssh-manager vm
use ssh_manager_core::{ExecutionOptions, OpenSsh, Store, SystemCredentials};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let helper = args
        .next()
        .ok_or("请提供 ssh-manager 可执行文件的绝对路径")?;
    let host = args.next().ok_or("请提供主机名称")?;
    let store = Store::open(Store::default_root()?)?;
    let credentials = SystemCredentials {
        helper_executable: helper.clone().into(),
    };
    let transport = OpenSsh {
        store: &store,
        credentials: &credentials,
        askpass_executable: helper.into(),
    };
    let options = ExecutionOptions {
        caller: "embedded-example".into(),
        ..Default::default()
    };
    let result = transport.check(&host, None, &options)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
