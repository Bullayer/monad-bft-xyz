
## 问题一：执行 command.sh 报错
错误信息：
```
`/Users/lewis/Documents/code/monad-bft-xyz/target/debug/monad-bull --validators-path monad-bull/config/validators.toml --secp-identity monad-bull/config/id-secp --bls-identity monad-bull/config/id-bls --node-config monad-bull/config/node.toml --forkpoint-config monad-bull/config/forkpoint.toml --wal-path monad-bull/config/wal --mempool-ipc-path monad-bull/config/mempool.sock --control-panel-ipc-path monad-bull/config/controlpanel.sock --ledger-path monad-bull/config/ledger --statesync-ipc-path monad-bull/config/statesync.sock`

error: secp secret must be encoded in keystore json
```

## 问题二：执行生成 keystore 报错
命令：
```
cd /Users/xxx/Documents/code/monad-bft-xyz/monad-scripts/keystore
bash generate_keystores.sh install
```
错误信息：
```
ERROR: Failed to build installable wheels for some pyproject.toml based projects (blspy)
Generating keystores
Traceback (most recent call last):
  File "/Users/xxx/Documents/code/monad-bft-xyz/monad-scripts/keystore/generate_keystores.py", line 4, in <module>
    from blspy import PrivateKey as BLSPrivateKey
ModuleNotFoundError: No module named 'blspy'
```
