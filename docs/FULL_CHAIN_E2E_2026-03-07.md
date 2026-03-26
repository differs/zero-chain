# Full-Chain E2E

## 目标

验证节点、矿池、矿工、浏览器在 ZeroChain 接口下可联通运行。

## 检查项

- [x] 组件健康检查通过
- [x] `zero_getLatestBlock` 区块高度增长
- [x] 矿池 shares 增长
- [x] `zero_getAccount` 返回规范 `ZER0x` 地址
- [x] Explorer 地址查询与搜索可用

## 关键 RPC

- `zero_getLatestBlock`
- `zero_getAccount`
- `zero_getWork`
- `zero_submitWork`

## 备注

该文档仅保留当前主路径结果；已移除的旧 transfer RPC 不再纳入检查项。
