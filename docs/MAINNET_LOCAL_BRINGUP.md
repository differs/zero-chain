# Mainnet Local Bring-up

用于本机单机预演以下组件：

- `bootnode`
- `follower`
- `observer`
- `zero-mining-stack pool`
- `zero-mining-stack miner`
- `zero-explore backend`

## 一键启动

```bash
cd zero-chain
bash scripts/mainnet_local_bringup.sh
```

默认会：

1. 停掉已有本地 `bootnode/follower/observer`
2. 启动 `bootnode`
3. 启动 `follower`
4. 启动 `observer`
5. 启动本地 pool
6. 启动本地 miner
7. 启动本地 explorer backend

## 默认地址

- `bootnode RPC`: `http://127.0.0.1:8545`
- `follower RPC`: `http://127.0.0.1:29645`
- `observer RPC`: `http://127.0.0.1:39745`
- `pool`: `http://127.0.0.1:9332`
- `miner metrics`: `http://127.0.0.1:9333`
- `explorer backend`: `http://127.0.0.1:19080`

## 启动后建议检查

```bash
./scripts/mainnet.sh status bootnode
./scripts/mainnet.sh status follower
./scripts/mainnet.sh status observer
bash scripts/mainnet_local_check.sh
curl -fsS http://127.0.0.1:9332/v1/stats
curl -fsS http://127.0.0.1:19080/api/overview
```

## 说明

- `bootnode` 使用：
  - `--mine`
  - `--disable-local-miner`
  - `--rpc-rate-limit-per-minute 0`
- 本地 miner 使用：
  - `--target-leading-zero-bytes 0`
- `bootnode` 日志会打印 `bootnode enode hint=...`
- follower / observer 使用该 `enode` 作为 bootnode

## 停止

```bash
cd zero-chain
bash scripts/mainnet_local_stop.sh
```
