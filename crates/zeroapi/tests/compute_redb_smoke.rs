use std::time::{SystemTime, UNIX_EPOCH};

use zeroapi::rpc::{ComputeBackend, JsonRpcRequest, RpcConfig, RpcServer};
use zerocore::compute::{Command, ComputeTx, DomainId, ObjectKind, ObjectReadRef, ObjectId, OutputId, OutputProposal, Ownership, TxId, TxSignature, TxWitness, Version};
use zerocore::crypto::{Hash, Signature};

fn parse_result(resp: &zeroapi::rpc::JsonRpcResponse) -> serde_json::Value {
    if let Some(result) = &resp.result {
        return result.clone();
    }
    panic!("result should exist, error: {:?}", resp.error);
}

#[tokio::test]
async fn compute_submit_result_output_smoke_redb_backend() {
    let mut path = std::env::temp_dir();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time ok")
        .as_millis();
    path.push(format!("zerochain-redb-smoke-{}.db", ts));

    let config = RpcConfig {
        compute_backend: ComputeBackend::Redb,
        compute_db_path: path.to_string_lossy().to_string(),
        ..RpcConfig::default()
    };

    let server = RpcServer::new(config);
    let api = server.api().expect("api should be initialized");

    let mut tx = ComputeTx {
        tx_id: TxId(Hash::from_bytes([0xC1u8; 32])),
        domain_id: DomainId(0),
        command: Command::Mint,
        input_set: vec![],
        read_set: Vec::<ObjectReadRef>::new(),
        output_proposals: vec![OutputProposal {
            output_id: OutputId(Hash::from_bytes([0xC2u8; 32])),
            object_id: ObjectId(Hash::from_bytes([0xC3u8; 32])),
            domain_id: DomainId(0),
            kind: ObjectKind::State,
            owner: Ownership::Shared,
            predecessor: None,
            version: Version(1),
            state: vec![0x01],
            logic: None,
        }],
        payload: vec![],
        deadline_unix_secs: None,
        chain_id: Some(10086),
        network_id: Some(1),
        witness: TxWitness {
            signatures: vec![TxSignature::secp256k1(Signature::new([1; 32], [2; 32], 27))],
            threshold: Some(1),
        },
    };
    tx.assign_expected_tx_id();
    let tx_id_hex = format!("0x{}", tx.tx_id.0.to_hex());
    let sig_hex = format!("0x{}", hex::encode(&tx.witness.signatures[0].bytes));

    let submit_req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "zero_submitComputeTx".to_string(),
        params: Some(vec![serde_json::json!({
            "tx_id": tx_id_hex.clone(),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
            "input_set": [],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xC2u8; 32])),
                "object_id": format!("0x{}", hex::encode([0xC3u8; 32])),
                "domain_id": 0,
                "kind": "State",
                "owner": { "type": "Shared" },
                "predecessor": null,
                "version": 1,
                "state": "0x01",
                "logic": null
            }],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {"signatures": [sig_hex], "threshold": 1}
        })]),
        id: serde_json::json!(1),
    };

    let submit_resp = api.handle_request(submit_req).await;
    let submit_result = parse_result(&submit_resp);
    assert_eq!(submit_result.get("ok").and_then(|v| v.as_bool()), Some(true));

    let query_resp = api
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "zero_getComputeTxResult".to_string(),
            params: Some(vec![serde_json::json!(tx_id_hex)]),
            id: serde_json::json!(2),
        })
        .await;
    let query_result = parse_result(&query_resp);
    assert_eq!(query_result.get("ok").and_then(|v| v.as_bool()), Some(true));
}
