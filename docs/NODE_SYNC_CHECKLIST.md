# 节点同步检查清单

适用范围：
- 本地主挖矿节点（默认 `127.0.0.1:19545`）
- 本地公网节点（默认 `127.0.0.1:29645`）
- 远端公网节点（默认 `139.180.207.66:28545`，通过 SSH 访问）

## 固定检查指标（每次都检查）

1. 进程与 RPC 可用性
- 主挖矿节点 RPC 可达。
- 本地公网节点 RPC 可达。
- 远端公网节点 RPC 可达。

2. 网络参数一致性
- 公网双节点 `net_version` 一致，且等于预期值（当前 `31337`）。

3. 对等连接健康
- 公网双节点 `net_peerCount >= 1`。
- 本地公网节点 `zero_peers` 包含远端端点（当前默认 `139.180.207.66:30303`）。

4. 区块同步一致性
- 公网双节点最新区块高度差 `<= 0`（即严格一致）。

5. 长稳监控状态（soak monitor）
- `LOCAL_OK=1`、`REMOTE_OK=1`。
- `LOCAL_NODE_ALIVE=1`、`REMOTE_NODE_ALIVE=1`。
- 错误计数均为 0：
  - `LOCAL_DROP_EVENTS`
  - `REMOTE_DROP_EVENTS`
  - `LOCAL_RPC_ERRORS`
  - `REMOTE_RPC_ERRORS`
  - `SSH_ERRORS`

## 一键执行

```bash
scripts/node_sync_check.sh
```

返回约定：
- Exit Code `0`：全部指标通过
- Exit Code `1`：存在失败项（输出中会标明 `[FAIL]`）

## 可调参数（环境变量）

```bash
EXPECTED_NET_VERSION=31337 \
MIN_PUBLIC_PEERS=1 \
MAX_PUBLIC_BLOCK_GAP=0 \
REMOTE_HOST=139.180.207.66 \
SSH_KEY=/root/.ssh/agent_139_180_207_66 \
scripts/node_sync_check.sh
```

