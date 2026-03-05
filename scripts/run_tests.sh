#!/bin/bash
# ZeroChain 测试运行脚本

set -e

echo "🧪 运行 ZeroChain 测试"
echo "======================"
echo ""

# 单元测试
echo "📋 运行单元测试..."
cargo test --lib -- --test-threads=1

echo ""
echo "📋 运行集成测试..."
cargo test --test integration_test

echo ""
echo "📊 测试覆盖率..."
cargo tarpaulin --out Html --output-dir ./coverage

echo ""
echo "✅ 所有测试完成!"
