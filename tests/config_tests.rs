use molenest::config::{Config, Defaults, ForwardPreset};
use molenest::ssh::build_ssh_command;

fn preset() -> ForwardPreset {
    ForwardPreset {
        name: "jupyter-8888".to_string(),
        host: "my-server".to_string(),
        local_port: 8888,
        remote_host: "127.0.0.1".to_string(),
        remote_port: 8888,
        bind_address: None,
        extra_args: vec![],
    }
}

#[test]
fn preset_lookup_by_name() {
    let config = Config {
        defaults: Defaults::default(),
        forwards: vec![preset()],
    };

    assert_eq!(
        config.find_preset("jupyter-8888").unwrap().host,
        "my-server"
    );
}

#[test]
fn invalid_port_is_rejected_by_deserializer() {
    let input = r#"
[[forwards]]
name = "bad"
host = "server"
local_port = 70000
remote_host = "127.0.0.1"
remote_port = 8888
"#;

    let result = toml::from_str::<Config>(input);
    assert!(result.is_err());
}

#[test]
fn command_preserves_ssh_host_alias() {
    let spec = build_ssh_command("ssh", &preset());

    assert_eq!(spec.args.last().unwrap(), "my-server");
    assert!(!spec.args.iter().any(|arg| arg.contains("example.com")));
}
