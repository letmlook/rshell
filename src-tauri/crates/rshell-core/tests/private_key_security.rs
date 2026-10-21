//! 切片 6.2：私钥永不过 IPC 安全断言（runtime）
//!
//! 设计 §4.2 "私钥 / 主密码"行 + §6.3 不变量"私钥永不过 IPC"边界。
//! 验证 `rshell_api::types::SshKeyInfo` 序列化输出不暴露私钥字段。
//!
//! 测试只读 `rshell_api::types` —— 这是边界层最严格的合约:
//! 任何新增敏感字段都会让本测试失败。

use rshell_api::types::{
    CredentialUpdate, Secret, SessionConfig, SessionCredential, SshKeyInfo, SshKeyType,
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

/// 全部**含密命令**的 Debug 都不带明文。
///
/// 旧用例只覆盖 `CreateSession`/`UpdateSession` 两种命令，恰好漏掉主密码与
/// 私钥口令那 6 个变体 —— 它们的 `password`/`passphrase` 曾是裸 `String`，
/// `#[instrument(fields(command = ?command))]` 会把明文写进 tracing span。
/// 现在这些字段是 `Secret`（Debug 恒为 `<redacted>`），本用例把覆盖面补齐，
/// 任何新增的含密变体也必须在这里登记。
#[test]
fn every_credential_bearing_command_redacts_its_secret() {
    let secret = "distinctive-secret-for-all-credential-commands";
    let commands = [
        (
            "SetupMasterPassword",
            AppCommand::SetupMasterPassword {
                password: Secret::new(secret),
            },
        ),
        (
            "VerifyMasterPassword",
            AppCommand::VerifyMasterPassword {
                password: Secret::new(secret),
            },
        ),
        (
            "ChangeMasterPassword",
            AppCommand::ChangeMasterPassword {
                old_password: Secret::new(secret),
                new_password: Secret::new(secret),
            },
        ),
        (
            "DisableMasterPassword",
            AppCommand::DisableMasterPassword {
                password: Secret::new(secret),
            },
        ),
        (
            "GenerateSshKey",
            AppCommand::GenerateSshKey {
                name: "k".into(),
                key_type: SshKeyType::ED25519,
                passphrase: Some(Secret::new(secret)),
            },
        ),
        (
            "ImportPrivateKey",
            AppCommand::ImportPrivateKey {
                path: std::path::PathBuf::from("/tmp/id_ed25519"),
                passphrase: Some(Secret::new(secret)),
            },
        ),
    ];
    for (name, command) in commands {
        let debug = format!("{command:?}");
        assert!(debug.contains(name), "{name} 变体名丢了: {debug}");
        assert!(
            debug.contains("<redacted>"),
            "{name} 的口令未被脱敏: {debug}"
        );
        assert!(
            !debug.contains(secret),
            "{name} 的口令经 Debug 泄漏: {debug}"
        );
    }
}

/// 命令分发处的日志只记**变体名**，不记负载 —— 负载里可能有终端输入、
/// 脚本源码、撰写文本，它们都可能含凭据。
#[test]
fn command_variant_name_carries_no_payload() {
    let secret = "distinctive-secret-for-variant-name";
    let command = AppCommand::SetupMasterPassword {
        password: Secret::new(secret),
    };
    let name = command.variant_name();
    assert_eq!(name, "SetupMasterPassword");
    assert!(!name.contains(secret));

    // 变体名必须覆盖到每一个命令变体：这里抽样断言几类代表性变体，
    // 完整覆盖由 `variant_name` 的穷尽匹配在编译期保证。
    assert_eq!(
        AppCommand::SendInput {
            session_id: Uuid::nil(),
            terminal_id: None,
            data: b"sudo -S hunter2".to_vec(),
        }
        .variant_name(),
        "SendInput"
    );
    assert_eq!(
        AppCommand::ExecuteScript {
            code: "rshell_log(\"hunter2\")".into(),
            session_id: Uuid::nil(),
        }
        .variant_name(),
        "ExecuteScript"
    );
}
