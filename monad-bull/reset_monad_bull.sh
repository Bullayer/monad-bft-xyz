#!/bin/bash

# monad-bull 重置脚本
# 清理临时文件并将 forkpoint.toml 重置为 genesis 状态

# 清理临时文件
rm -rf ./config/forkpoint.rlp.*
echo "✅ 清理 forkpoint.rlp 完成！"

rm -rf ./config/forkpoint.toml.*
echo "✅ 清理 forkpoint.toml 完成！"

rm -rf ./config/validators.toml.*
echo "✅ 清理 validators.toml 完成！"

rm -rf ./config/wal_*
echo "✅ 清理 wal 完成！"

rm -rf ./config/ledger/*
echo "✅ 清理 ledger 完成！"

# 重置 root 字段
sed -i '' 's|^root = ".*"|root = "0x0000000000000000000000000000000000000000000000000000000000000000"|' ./config/forkpoint.toml

# 重置 signatures 字段
sed -i '' 's|^signatures = ".*"|signatures = "0xf865c28080b860c00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"|' ./config/forkpoint.toml

# 重置 high_certificate.Qc.info.id 字段
sed -i '' 's|^id = ".*"|id = "0x0000000000000000000000000000000000000000000000000000000000000000"|' ./config/forkpoint.toml

# 重置 high_certificate.Qc.info.round 字段
sed -i '' 's|^round = .*|round = 0|' ./config/forkpoint.toml

echo "✅ 重置 forkpoint.toml 完成！"