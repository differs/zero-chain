# zerostore

ZeroChain 存储层（数据库、索引与 Trie 等）。

## Bench

仓库内的 `benches/trie_bench.rs` 也会复用该 crate 的实现：

```bash
cargo bench -p zerostore trie_bench
```

## 本地开发

在仓库根目录执行：

```bash
cargo test -p zerostore
```

