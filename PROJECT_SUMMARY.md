# ZeroChain 项目摘要（Native-Only）

## 当前基线

- 共识：PoW
- 执行：UTXO Compute
- 签名：`ed25519`
- 节点接口：`web3_*` / `net_*` / `zero_*`
- CLI：钱包、账户、交易、区块、RPC、节点运行

## 已完成

- 原生交易提交与结果查询链路
- 钱包与 CLI 统一 native 地址与签名流程
- 矿池/矿工与节点联调
- Explorer 与节点 native 地址语义对齐

## 进行中

- 全链路压测与性能调优
- 更完整的生产观测与告警模板
- 释放流程自动化（发布门禁、回归矩阵）

## 文档入口

- `README.md`
- `ARCHITECTURE.md`
- `docs/GETTING_STARTED.md`
- `docs/API.md`
