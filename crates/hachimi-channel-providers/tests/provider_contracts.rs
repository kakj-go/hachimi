use hachimi_channel_providers::{
    DingTalkAdapter, FeishuAdapter, ProviderAdapter, ProviderEventFrame, TransportProof,
    WechatIlinkAdapter, WecomAiBotAdapter, WecomAppAdapter,
};

fn frame(payload: serde_json::Value, proof: TransportProof) -> ProviderEventFrame {
    ProviderEventFrame {
        account_id: "account-1".into(),
        tenant_key: "tenant-1".into(),
        payload,
        proof,
    }
}

fn stream() -> TransportProof {
    TransportProof::Stream {
        connection_id: "connection-1".into(),
        received_at_ms: 10,
    }
}

#[test]
fn feishu_never_uses_message_id_as_topic() {
    let message = FeishuAdapter::default()
        .normalize(frame(
            serde_json::json!({
                "event": {
                    "sender": {"sender_id": {"open_id": "user-1"}},
                    "message": {"message_id": "message-1", "chat_id": "chat-1", "chat_type": "group", "root_id": "message-1", "text": "hello"}
                }
            }),
            stream(),
        ))
        .expect("message");
    assert_eq!(message.address.chat_id, "chat-1");
    assert_eq!(message.address.topic_id, None);
}

#[test]
fn all_five_adapters_produce_stable_dm_or_group_addresses() {
    let ding = DingTalkAdapter
        .normalize(frame(
            serde_json::json!({"msgId":"d1","senderStaffId":"u1","conversationType":"1","text":{"content":"hi"}}),
            stream(),
        ))
        .expect("dingtalk");
    let ai = WecomAiBotAdapter
        .normalize(frame(
            serde_json::json!({"msgid":"w1","userid":"u2","text":{"content":"hi"}}),
            stream(),
        ))
        .expect("wecom ai");
    let app = WecomAppAdapter
        .normalize(frame(
            serde_json::json!({"MsgId":"a1","FromUserName":"u3","Content":"hi"}),
            TransportProof::SignedCallback {
                signature_fingerprint: "sha256:x".into(),
                received_at_ms: 10,
            },
        ))
        .expect("wecom app");
    let ilink = WechatIlinkAdapter
        .normalize(frame(
            serde_json::json!({
                "message_id":"i1",
                "message_type": 1,
                "from_user_id":"u4",
                "context_token":"ctx",
                "bot_id":"bot",
                "item_list":[{"type":1,"text_item":{"text":"hi"}}]
            }),
            TransportProof::IlinkPoll {
                bot_id: "bot".into(),
                received_at_ms: 10,
            },
        ))
        .expect("ilink");
    assert_eq!(
        [ding, ai, app, ilink].map(|message| message.address.topic_id),
        [None, None, None, None]
    );
}

#[test]
fn ilink_rejects_untrusted_hosts() {
    assert!(WechatIlinkAdapter::validate_base_url("https://ilinkai.weixin.qq.com").is_ok());
    assert!(WechatIlinkAdapter::validate_base_url("https://example.com").is_err());
}
