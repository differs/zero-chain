# ZeroChain Storage Architecture

This document defines the storage direction for ZeroChain nodes after the initial RocksDB/Redb
compute store implementation.

## Goals

- Keep consensus and UTXO Compute validation on low-latency key-value storage.
- Reduce hot node disk footprint without making random output lookups expensive.
- Move historical scan/analytics workloads out of the consensus hot path.
- Preserve forward-compatible on-disk codecs so nodes can read older local databases.

## Data Tiers

### Hot State Tier

Hot state is required to validate new blocks and compute transactions.

- Backend: RocksDB for production, Redb for portable/local testing, Mem for tests.
- Access pattern: point lookups by `OutputId`, latest output lookup by `ObjectId`, small writes per transaction.
- Format: versioned compact binary values.
- Compression: RocksDB ZSTD by default.
- Retention: live outputs and recent state needed for rollback/reorg windows.

The hot tier must not use Parquet/ORC-style columnar files because validation needs random lookups
and atomic updates. Columnar files are optimized for scans, not UTXO point reads.

### Historical Archive Tier

Historical archive data is not needed for immediate validation after finalization.

- Data: old blocks, spent outputs, old compute results, historical receipts, explorer operation rows.
- Format: segment files by height range, then compressed with ZSTD.
- Access pattern: append by segment, fetch by height/hash, rare reindex.
- Retention: configurable per node profile.

Archive data can be pruned on ordinary full nodes and retained on archive/explorer nodes.

### Columnar Analytics Tier

Columnar storage belongs to explorer and analytics workloads.

- Data: address histories, operation lists, block summaries, compute result summaries.
- Format: Parquet/Arrow-style schema outside the consensus hot DB.
- Compression: ZSTD with dictionary/statistics when enough samples exist.
- Access pattern: scans, filters, aggregations.

This tier is rebuildable from archive/hot data. It must not be a source of consensus truth.

## Current Implementation Step

The first code step changes compute outputs from JSON values to versioned binary values:

```text
key   = "compute:output:" || output_id
value = "ZCO1" || bincode(ObjectOutput)
```

Readers keep JSON fallback support so existing development databases remain readable. New writes use
binary. This removes repeated JSON field names and decimal byte-array formatting, which are the
largest avoidable overhead in the current compute store.

Compute tx results remain JSON for now because the RPC result is still dynamic and mostly used as
API-facing metadata. They should be converted later to a typed result envelope:

```text
key   = "compute:txresult:" || tx_id
value = "ZCR1" || bincode(ComputeResultEnvelope)
```

## Expected Savings

The practical savings are workload-dependent:

- JSON compute outputs to binary values: roughly 30-50% smaller for output-heavy workloads.
- RocksDB LZ4 to ZSTD: usually lower disk usage, with higher CPU cost.
- Hot state plus pruning/snapshots: 70-95% smaller than archive-style full history nodes.
- Columnar analytics: strong savings for scans and repeated fields, but not for hashes/signatures.

Hash-like fields (`OutputId`, `TxId`, block hashes, signatures, public keys) are high entropy and do
not compress much. Major gains come from avoiding JSON overhead, pruning history, and separating
analytics data from validation data.

## Node Profiles

### Full Validator

- Keeps hot state and recent rollback window.
- Uses RocksDB ZSTD and binary compute output values.
- Prunes finalized historical compute outputs unless configured otherwise.

### Archive Node

- Keeps hot state plus historical archive segments.
- May store block/compute history in compressed segment files.
- Can rebuild columnar analytics exports.

### Explorer Node

- Reads RPC/archive feeds.
- Builds columnar analytics tables for address, block, and compute browsing.
- Does not participate in consensus validation from columnar data.

## Migration Rules

- On-disk values must carry a codec magic/version prefix when they are not legacy JSON.
- Readers should support at least the previous production codec during development.
- New writers may use the latest codec immediately if the reader is backward compatible.
- Destructive rewrites or pruning need an explicit operator command and a rollback plan.

Compute DB rebuilds are explicit operator actions. They rewrite output values through the current
codec and recreate file-based backends with the active backend options, including RocksDB ZSTD:

```bash
zerochain --network mainnet storage rebuild-compute-db
zerochain storage rebuild-compute-db \
  --compute-backend rocksdb \
  --compute-db-path ./data/compute-db
```

The node must be stopped before running a rebuild; file-based backends take an exclusive database
lock and the command intentionally refuses unsafe in-memory rebuilds.

The command writes a new database at `<path>.rebuild-<timestamp>`, moves the old database to
`<path>.backup-<timestamp>`, then installs the rebuilt database at the original path. Keep the backup
until the node has restarted and caught up successfully; pass `--discard-backup` only for disposable
development databases.

## Next Steps

- Add typed binary codec for tx results.
- Add optional pruning policy for spent outputs and old tx results.
- Add archive segment writer for finalized block ranges.
- Add rebuildable Parquet/Arrow export for explorer analytics.
