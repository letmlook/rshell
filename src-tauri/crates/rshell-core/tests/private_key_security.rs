//! 切片 6.2：私钥永不过 IPC 安全断言（runtime）
//!
//! 设计 §4.2 "私钥 / 主密码"行 + §6.3 不变量"私钥永不过 IPC"边界。
//! 验证 `rshell_api::types::SshKeyInfo` 序列化输出不暴露私钥字段。
//!
//! 测试只读 `rshell_api::types` —— 这是边界层最严格的合约:
//! 任何新增敏感字段都会让本测试失败。

use rshell_api::types::{
    CredentialUpdate, SessionConfig, SessionCredential, SshKeyInfo, SshKeyType,
};
use rshell_api::AppCommand;
use uuid::Uuid;

fn sample_info() -> SshKeyInfo {
    SshKeyInfo {
        id: Uuid::nil(),
        name: "test".into(),
        key_type: SshKeyType::ED25519,
        fingerprint: "SHA256:abc123".into(),
        public_key_blob: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...".into(),
        comment: "test@host".into(),
        has_passphrase: true,
        created_at: "2026-07-31T00:00:00Z".into(),
    }
}

#[test]
fn ssh_key_info_json_never_contains_private_key() {
    let json = serde_json::to_string(&sample_info()).expect("serialize");
    let lower = json.to_lowercase();
    // 先剥离 "has_passphrase" 字段名（仅 bool 标志位,设计 §4.2 "私钥 / 主密码"行允许）
    let stripped = lower.replace("has_passphrase", "");
    assert!(
        !stripped.contains("private"),
        "SshKeyInfo JSON must not expose any private-key field; got: {}",
        json
    );
    assert!(
        !stripped.contains("secret"),
        "SshKeyInfo JSON must not expose any secret field; got: {}",
        json
    );
    assert!(
        !stripped.contains("passphrase"),
        "SshKeyInfo JSON must not expose passphrase (only has_passphrase bool flag); got: {}",
        json
    );
    assert!(
        !stripped.contains("pem"),
        "SshKeyInfo JSON must not expose PEM-encoded data; got: {}",
        json
    );
}

#[test]
fn ssh_key_info_field_set_is_whitelisted() {
    let v: serde_json::Value = serde_json::to_value(sample_info()).unwrap();
    let obj = v.as_object().expect("SshKeyInfo must serialize as object");
    let allowed: std::collections::HashSet<&str> = [
        "id",
        "name",
        "key_type",
        "fingerprint",
        "public_key_blob",
        "comment",
        "has_passphrase",
        "created_at",
    ]
    .iter()
    .copied()
    .collect();
    for key in obj.keys() {
        assert!(
            allowed.contains(key.as_str()),
            "SshKeyInfo has unexpected field `{}` (potential private-key leak)",
            key
        );
    }
    assert_eq!(
        obj.len(),
        allowed.len(),
        "SshKeyInfo field count drifted from security whitelist"
    );
}

#[test]
fn session_config_json_never_contains_password_or_passphrase() {
    for auth_method in [
        serde_json::json!({"Password": {"username": "alice", "has_password": true, "password": "sample-secret-password"}}),
        serde_json::json!({"PublicKey": {"username": "alice", "key_path": "/tmp/id_ed25519", "has_passphrase": true, "passphrase": "sample-secret-passphrase"}}),
        serde_json::json!({"KeyboardInteractive": {"username": "alice", "has_password": true, "password": "sample-secret-interactive"}}),
    ] {
        let config: SessionConfig = serde_json::from_value(serde_json::json!({
            "id": Uuid::nil(),
            "name": "sample",
            "folder_id": null,
            "host": "example.test",
            "port": 22,
            "protocol": "SSH",
            "auth_method": auth_method,
            "serial_config": null,
        }))
        .unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let auth = serde_json::to_value(&config).unwrap()["auth_method"].clone();
        let descriptor = auth
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .as_object()
            .unwrap();
        assert!(
            !descriptor.contains_key("password"),
            "SessionConfig exposed password: {json}"
        );
        assert!(
            !descriptor.contains_key("passphrase"),
            "SessionConfig exposed passphrase: {json}"
        );
        assert!(
            !json.contains("sample-secret"),
            "SessionConfig exposed a secret: {json}"
        );
    }
}

#[test]
fn credential_bearing_commands_redact_secrets_in_debug_output() {
    let config: SessionConfig = serde_json::from_value(serde_json::json!({
        "id": Uuid::nil(), "name": "sample", "folder_id": null,
        "host": "example.test", "port": 22, "protocol": "SSH",
        "auth_method": {"Password": {"username": "alice", "has_password": true}},
        "serial_config": null,
    }))
    .unwrap();
    let secret = "distinctive-secret-for-debug-regression";
    let commands = [
        AppCommand::CreateSession {
            config: config.clone(),
            credential: Some(SessionCredential {
                secret: secret.into(),
            }),
        },
        AppCommand::UpdateSession {
            id: config.id,
            config,
            credential: CredentialUpdate::Set(SessionCredential {
                secret: secret.into(),
            }),
        },
    ];
    for (command, name) in commands.into_iter().zip(["CreateSession", "UpdateSession"]) {
        let debug = format!("{command:?}");
        assert!(debug.contains(name));
        assert!(debug.contains("<redacted>"));
        assert!(
            !debug.contains(secret),
            "credential leaked through command Debug: {debug}"
        );
    }
}
