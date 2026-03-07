# ZeroChain Observability (Tracing + Metrics)

本项目当前已支持：

1. **Tracing (OTel)**：`zerochain` 可将 tracing 导出到 OTLP。
2. **Metrics (Prometheus text via RPC)**：`zero_getMetrics` 返回 Prometheus 格式文本。

---

## 1) 启动本地 OTel Collector + Jaeger

```bash
cd deploy/observability
docker compose up -d
```

查看 Jaeger UI：

- http://127.0.0.1:16686

---

## 2) 以 OTel 模式启动节点

```bash
zerochain --otel-enabled --otel-endpoint http://127.0.0.1:4317 --network testnet run
```

你也可以配合脚本：

```bash
scripts/testnet.sh start --nodes 3 --clean-data
```

---

## 3) 查看节点指标

调用 RPC 方法 `zero_getMetrics`：

```bash
curl -s http://127.0.0.1:8545 \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"zero_getMetrics","params":[]}' | jq -r '.result.text'
```

关键指标：

- `zero_rpc_method_calls_total{method="..."}`
- `zero_rpc_method_errors_total{method="..."}`
- `zero_mining_shares_accepted_total{source="zero_submitWork"}`
- `zero_mining_shares_rejected_total{reason="..."}`
- `zero_latest_block_height`

---

## 4) 主网建议

主网上建议同时启用：

- Traces（OTLP -> Jaeger/Tempo）
- Metrics（Prometheus 抓取）
- 结构化日志（包含 trace_id/span_id）

并配置告警：

- submitWork reject rate 异常上升
- block height 停滞
- RPC error rate > 阈值

---

## 5) 公网 P2P 长稳监控（Soak）

当需要验证公网节点长期连通（例如连续 72 小时）时，使用脚本：

```bash
scripts/public_node_soak_monitor.sh start
```

默认行为：

- 每 `60s` 采样一次
- 连续运行 `72h`
- 同时采集本地节点（`http://127.0.0.1:29645`）与远端节点（通过 SSH 在远端访问 `127.0.0.1:28545`）的 `zero_peers`/`web3_clientVersion` 健康数据
- 输出 CSV 到 `artifacts/public-node-soak-monitor/<timestamp>/samples.csv`

常用命令：

```bash
scripts/public_node_soak_monitor.sh status
scripts/public_node_soak_monitor.sh logs
scripts/public_node_soak_monitor.sh stop
```
