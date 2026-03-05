#!/bin/bash
# ZeroChain 性能基准测试脚本

set -e

echo "⚡ 运行 ZeroChain 基准测试"
echo "========================="
echo ""

# 运行基准测试
echo "📊 运行基准测试..."
cargo bench -- --output-format bencher | tee benchmark_results.txt

echo ""
echo "✅ 基准测试完成!"
echo "📁 结果保存在：benchmark_results.txt"
