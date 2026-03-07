# Full-Chain E2E（Native-Only）

## 目标

验证节点、矿池、矿工、浏览器在 native 接口下可联通运行。

## 检查项

- [x] 组件健康检查通过
- [x] `zero_getLatestBlock` 区块高度增长
- [x] 矿池 shares 增长
- [x] `zero_getAccount` 返回规范 `ZER0x` 地址
- [x] `zero_transfer` 账户间余额变动成功
- [x] Explorer 地址查询与搜索可用

## 关键 RPC

- `zero_getLatestBlock`
- `zero_getAccount`
- `zero_transfer`
- `zero_getWork`
- `zero_submitWork`

## 备注

该文档仅保留 native 主路径结果；历史兼容路径记录已下线。
