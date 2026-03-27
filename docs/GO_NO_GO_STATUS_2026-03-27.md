# ZeroChain GO / NO-GO Status (2026-03-27)

## 结论

- 当前结论：`NO-GO`
- 适用范围：主网正式上线、真实用户、真实资产、公开真实挖矿

可以明确的是：

- 当前代码与跨仓主链路已经具备“可启动、可联通、可做封闭预演/灰度 rehearsal”的能力。
- 当前状态不满足仓内既有发布门禁，不能定义为正式主网可上线状态。

## 已有正向证据

### 自动化与联通

- `artifacts/release-gate/go-no-go-report.md`
  当前自动化门禁记录为：
  - `cargo fmt --all --check` PASS
  - `cargo check --workspace` PASS
  - `cargo test --workspace` PASS
  - 结论仍为 `NO-GO until manual blocking items close`

- `docs/FULL_CHAIN_E2E_2026-03-07.md`
  已有跨组件主路径联通证据：
  - 节点健康
  - `zero_getLatestBlock` 高度增长
  - 矿池 shares 增长
  - `zero_getAccount` 可用
  - explorer 地址查询与搜索可用

- `docs/WORKSPACE_ACCEPTANCE_CHECKLIST.md`
  统一验收入口已经固化：
  - `bash scripts/workspace_acceptance.sh --quick`
  - `bash scripts/workspace_acceptance.sh --full`

### 当前实测状态

截至 2026-03-27，本轮额外确认了以下事实：

- `zero-explore` 的 `/api/blocks/recent` 已恢复可用。
- 外部矿工 smoke 已可稳定跑通：
  - 节点可用 `--disable-local-miner`
  - 可显式覆盖 `zero_getWork` 难度
  - 本地 smoke 可显式关闭 RPC rate limit
- `zero-mining-stack` 单节点本地模式下，mirror peers 已改为 opt-in，不再默认刷错误日志。
- `zero-wallet-chrome` 已通过构建、单测和扩展 UI smoke。
- `zero-wallet-mobile` 已通过 `flutter analyze` / `flutter test`，但 GUI 真启动仍受当前环境限制。

## 按阻塞项的当前判断

下面按 `docs/GO_NO_GO_CHECKLIST.md` 和 `docs/P0_RELEASE_BLOCKERS_2026-03.md` 归类。

### A. 代码与构建

- A1 版本 tag 与制品一一对应：`FAIL`
  现状：
  - 当前没有看到本轮发布结论所绑定的 release tag 证据。

- A2 `cargo check --workspace`：`PASS`
  证据：
  - `artifacts/release-gate/go-no-go-report.md`

- A3 `cargo test --workspace`：`PASS`
  证据：
  - `artifacts/release-gate/go-no-go-report.md`

- A4 关键 crate 热路径人工审查：`FAIL`
  现状：
  - 仓内没有看到完成态审查记录或签署结论。

### B. 协议与交易安全

- B1 签名绑定完整性：`PASS`
- B2 `tx_id == signing_preimage` 校验：`PASS`
- B3 `Ownership::Address` 强制归属校验：`PASS`
- B4 重放风险边界确认：`PARTIAL / 视作 FAIL`
  原因：
  - 代码侧已有 chain/network/domain 绑定实现，但没有看到发布级确认记录与完整测试签署。

- B5 授权失败错误码 3001~3005：`PASS`
  依据：
  - 现有测试与前几轮重构已覆盖结构化错误路径。

### C. 状态与存储一致性

- C1 持久化后重启恢复一致：`PASS`
  依据：
  - compute smoke / redb smoke 已存在

- C2 幂等行为稳定：`PASS`
  依据：
  - 现有 compute / submit / duplicate 路径已有测试

- C3 目标环境数据库后端验证：`PARTIAL / 视作 FAIL`
  原因：
  - 本地与 smoke 后端可跑，不等于生产目标环境已验证完成

- C4 数据目录权限、容量阈值、备份策略：`FAIL`
  原因：
  - 仓内未见落实证据

### D. RPC/API 稳定性

- D1 核心 RPC 冒烟：`PASS`
- D2 错误码与 API 文档一致：`PASS`
- D3 向后兼容评估完成：`PARTIAL / 视作 FAIL`
  原因：
  - 近期已进行大量命名与接口收敛，但缺少正式兼容影响评估记录

### E. 安全与合规

- E1 最小安全审计：`FAIL`
  依据：
  - `docs/P0_RELEASE_BLOCKERS_2026-03.md` 当前仍为 `TODO`

- E2 无明文密钥/敏感配置入仓：`PARTIAL`
  说明：
  - 没看到当前违规证据，但也没有正式扫描报告，因此发布口径不能直接记 `PASS`

- E3 生产密钥托管与轮换：`FAIL`
  依据：
  - `docs/P0_RELEASE_BLOCKERS_2026-03.md` 当前仍为 `TODO`

### F. 运维与可观测性

- F1 指标可观测：`IN_PROGRESS / 视作 FAIL`
- F2 日志可追踪：`IN_PROGRESS / 视作 FAIL`
- F3 告警演练：`FAIL`
- F4 回滚演练：`FAIL`

依据：

- `docs/P0_RELEASE_BLOCKERS_2026-03.md` 当前状态仍未关闭

### G. 性能与容量

- G1 压测阈值：`FAIL`
- G2 24h/72h Soak：`FAIL`
- G3 峰值可恢复：`FAIL`

依据：

- `docs/P0_RELEASE_BLOCKERS_2026-03.md` 当前状态仍为 `TODO`
- `docs/GO_NO_GO_CHECKLIST.md` 也明确把 24h 稳定性运行列为最低 GO 条件之一

## 为什么当前不是 GO

不是因为“链路跑不起来”，而是因为“发布阻断项没关完”。

当前已经能做到：

- mainnet profile 可启动
- 节点 / explorer / mining stack / 钱包主路径基本可联通
- 外部矿工真实 smoke 可跑

但仍然缺：

1. 安全审计结论
2. 生产密钥托管与轮换演练
3. 告警演练
4. 回滚演练
5. 压测报告
6. 24h/72h soak 报告
7. 峰值故障恢复报告
8. 发布 tag 与正式制品绑定

以上任一类都不是“可选润色项”，而是你们自己文档里定义的阻断项。

## 当前建议

- 对外正式主网上线：`NO`
- 真实公开挖矿：`NO`
- 封闭环境 / 小范围灰度 rehearsal：`YES`
- 单节点或少节点预演、发布演练、运维演练：`YES`

## 建议的下一步顺序

1. 先完成 `E1` 安全审计与扫描报告
2. 紧接着完成 `E3` 生产密钥托管/轮换演练
3. 完成 `F3/F4` 告警与回滚演练
4. 跑 `G2` 至少 24h soak
5. 输出 `G1/G3` 压测与故障恢复报告
6. 最后再开一次正式 Go/No-Go 评审，并绑定版本 tag 与制品哈希
