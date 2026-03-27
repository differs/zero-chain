# Workspace Acceptance Checklist

这份清单用于统一验收当前工作区的 5 个主仓：

- `zero-chain`
- `zero-explore`
- `zero-mining-stack`
- `zero-wallet-chrome`
- `zero-wallet-mobile`

优先入口脚本：

```bash
cd zero-chain
bash scripts/workspace_acceptance.sh
```

## 自动化门禁

以下项目应由统一脚本自动完成，并全部通过：

1. `zero-chain/scripts/full_chain_e2e.sh`
   预期：
   - 节点可启动
   - 外部矿池/矿工可联通
   - explorer backend/frontend 可启动
   - 区块高度增长
   - pool shares >= 1
   - explorer 账户与搜索接口可用

2. `zero-mining-stack/scripts/nightly_local_qa.sh`
   预期：
   - 节点以外部矿工 smoke 模式启动
   - pool / miner 健康检查通过
   - `zero_getWork` / `zero_submitWork` 联通
   - accepted shares >= 1

3. `zero-wallet-chrome`
   命令：
   - `bun run build`
   - `bun run test`
   - `bun run qa:extension`
   预期：
   - 扩展可构建
   - 单测通过
   - onboarding / home / receive / send / settings smoke 通过

4. `zero-wallet-mobile`
   命令：
   - `flutter analyze`
   - `flutter test`
   - `flutter devices`
   预期：
   - analyze 无错误
   - 测试通过
   - 至少能列出当前可用设备

## 人工复核

以下项目受当前机器环境影响，默认不强制自动化：

1. `zero-wallet-mobile` 真机或桌面 UI 启动
   命令：
   - `flutter run -d linux`
   - 或 `flutter run -d <android-device-id>`
   预期：
   - 应用可拉起
   - 创建钱包 / 导入钱包 / 发送页可进入

2. `zero-wallet-mobile` Web 启动
   命令：
   - `flutter run -d chrome`
   说明：
   - 若当前环境是 root + Chrome sandbox 限制，允许记录为环境阻塞

3. `zero-mining-stack` block mirror
   说明：
   - 默认不启用 mirror
   - 只有显式传入 `--mirror-peer <rpc-url>` 时才验收多节点镜像行为

## 当前设计约束

1. 外部矿工 smoke 应显式使用：
   - `zerochain run --mine --disable-local-miner`
   - `--mining-work-target-leading-zero-bytes 1`
   - `--rpc-rate-limit-per-minute 0`

2. 本地 miner smoke 应显式使用：
   - `zero-mining-stack miner --target-leading-zero-bytes 0`

3. 单节点本地模式下：
   - pool 默认不做 mirror
   - 不应再出现对 `127.0.0.1:8546/8547` 的持续 mirror 报错
