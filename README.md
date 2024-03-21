# linea-sgx-prover

## RoadMap

- [x] Generate Proof of Block (Pob) in SGX prover
- [x] Build Geth-compatible blocks with Pob in SGX prover
- [x] DCAP Attestation On-chain verification
- [x] SGX prover registry & key rotation
- [x] SGX prover submits rollup proofs
- [x] Implement the Rust version of Linea zktrie that aligns with the new spec [shomei](https://github.com/Consensys/shomei)
- [ ] Integrate with the new zktrie to the SGX prover and generate the root_hash (working in progress)
- [ ] Submit the root_hash of zktrie to the SGX Verifier


## Getting Started

### 1. System Dependencies

Install node using nvm. It's recommended to use v18.16.1. Then, install hardhat.

```
> nvm install v18.16.1
> nvm use v18.16.1
> cd verifier && npm install
```

Initialize the SGX environment. You can refer [here](https://github.com/automata-network/attestable-build-tool/blob/main/image/rust/Dockerfile).
It's recommended to use Azure with the `Standard DC4s v3` size.

Install the latest [Geth](https://github.com/ethereum/go-ethereum). We'll be using it to launch a local node as the L1 node and deploy the verifier contract.   
Ensure it's added to the PATH environment variable.

### 2. Environment Initialization

#### 2.1. Deploy Contract

```
> CHAIN_ID=59140 URL=https://goerli.infura.io/v3/{API_KEY} ./scripts/verifier.sh deploy
attestation address: 0xf9fE0D45f4D2E6039a13cBC9aFAc292140379112
verifier address: 0xBf2A60958a0dF024Ffa1dF8C652240C42425762c
```

#### 2.3. Configure the prover

`config/prover-goerli.json`:  
```json
{
    "l2": "http://localhost:8545",
    "verifier": {
        "endpoint": "https://goerli.infura.io/v3/{API_KEY}",
        "relay_account": "0x135e5f68224c169b016d92aedb6af6163e6d985dd6d25b3bbd1124e964490843", <- Do not modify in the test environment
        "contract": "0x5431a78B73A59dA94Db1aB91473698435C80AE80" <- Replace with the deployed verifier contract address
    },
    "rollup": {
        "endpoint": "https://goerli.infura.io/v3/{API_KEY}",
        "contract": "0x70BaD09280FD342D02fe64119779BC1f0791BAC2"
    },
    "server": {
        "tls": "",
        "body_limit": 2097152,
        "workers": 10
    }
}
```

### 3. Test

#### 3.1. Run the prover

Open a terminal window and execute:
```bash
> RELEASE=1 NETWORK=goerli ./script/prover.sh
[2024-01-29 08:54:50.451] [e473a727ce493a65] [sgx_prover_enclave:25] [INFO] - Initialize Enclave!
[2024-01-29 08:54:50.451] [e473a727ce493a65] [app:157] [INFO] - args: "[\"bin/sgx/target/release/sgx-prover\",\"-c\",\"config/prover-goerli.json\"]"
[2024-01-29 08:54:51.661] [e473a727ce493a65] [linea::prover:25] [INFO] - prover pubkey: 0x6068bad509a29b056b0db10689ffb53cb7ba14ce
[2024-01-29 08:54:52.152] [prover-attested-monitor] [linea::prover:130] [INFO] - getting prover[0xcdeb09d785f58865782c3856abff45b7f45b4c8e] attested...
[2024-01-29 08:54:52.884] [e473a727ce493a65] [eth_tools::eth_log_subscriber:38] [WARN] - [batch-task] incorrect start offset=0, head=10450728, reset to head
[2024-01-29 08:54:53.555] [prover-attested-monitor] [linea::verifier:132] [INFO] - [submit_attestation_report]tx sent: TxReceipt { addr: 0x60c694caca23dba388b529920280235530399a7e, seq: 0, receiver: Receiver { .. }, status: Sent((0x05daf3f8ce843b18880325ea685e5b74b9350e7aa83d9e46cffc4d086769ec82, None)) }
[2024-01-29 08:54:59.716] [TxSender] [eth_tools::tx_sender:270] [INFO] - checking tx receipt: 0x05daf3f8ce843b18880325ea685e5b74b9350e7aa83d9e46cffc4d086769ec82, live_time: Some(6.160529427s)
[2024-01-29 08:55:00.961] [TxSender] [eth_tools::tx_sender:270] [INFO] - checking tx receipt: 0x05daf3f8ce843b18880325ea685e5b74b9350e7aa83d9e46cffc4d086769ec82, live_time: Some(7.405551232s)
[2024-01-29 08:55:01.207] [prover-attested-monitor] [linea::verifier:135] [INFO] - [submit_attestation_report] tx updates: Confirmed(Receipt { type: Some(0), root: None, status: 1, cumulative_gas_used: 12476453, logs_bloom: 0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000080000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000010000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000, logs: [Log { address: 0x5431a78b73a59da94db1ab91473698435c80ae80, topics: [0xc22b5a1aaf7898c383e1cb59eceb536799739c02ec9c197a3bdad2ce5abed055], data: 0x000000000000000000000000cdeb09d785f58865782c3856abff45b7f45b4c8e, block_number: 10450734, transaction_hash: 0x05daf3f8ce843b18880325ea685e5b74b9350e7aa83d9e46cffc4d086769ec82, transaction_index: 29, block_hash: 0x49ba95008b5ba12baac8290fa6eab53aa440f6404ff6f07321498c8501016c43, log_index: 54, removed: false }], transaction_hash: 0x05daf3f8ce843b18880325ea685e5b74b9350e7aa83d9e46cffc4d086769ec82, contract_address: None, gas_used: 5872926, block_hash: Some(0x49ba95008b5ba12baac8290fa6eab53aa440f6404ff6f07321498c8501016c43), block_number: Some(10450734), transaction_index: 29 })
[2024-01-29 08:55:02.207] [prover-attested-monitor] [linea::prover:146] [INFO] - attestation report submitted -> 0xcdeb09d785f58865782c3856abff45b7f45b4c8e

[2024-01-29 08:55:07.698] [prover-attested-monitor] [linea::prover:109] [INFO] - prover[0xcdeb09d785f58865782c3856abff45b7f45b4c8e] is attested...
```
Wait for "prover[..] is attested" to appear.

#### 3.2. Configure the MrEnclave/MrSigner validation

The MrEnclave/MrSigner validation is disabled by default for testing. It can be enable by calling the `toggleLocalReportCheck()`.

If you seeing the log like "generate report fail" as below, It means the mrenclave and mrsigner hasn't registered.
```
[2023-12-05 15:09:23.317] [prover-status-monitor] [app_prover::app:117] [INFO] - mrenclave: 0x0cfc69503cfe2f072431f3776f19a03253873f0c4a86d65dda8d16ec48e207f8, mrsigner: 0x0a9cc8e1d9a313accdf197223f876214f6869c42f2fa5d92f13290b09e9a5b4b
[2023-12-05 15:09:23.317] [prover-status-monitor] [prover::prover:297] [INFO] - generate report fail: mrenclave[false] or mr_signer[false] not trusted
[2023-12-05 15:09:24.807] [prover-status-monitor] [prover::prover:293] [INFO] - getting prover[0xc181b7acf902231b032821e7fb6c98827d3c8cb4] attested...
```

Register the mrenclave and mrsigner to the chain
```
> CONTRACT=0xf9fE0D45f4D2E6039a13cBC9aFAc292140379112 MRENCLAVE=0xebef290e360154b0ff307d0734ec62687bbc863353911a171b5fdd51484fc81f MRSIGNER=0xdb00409d350dc9705d2f6a1c76184341eb63d48c68d95077f2d477e426a73622 ./scripts/verifier.sh add_mrenclave
```

#### 3.3. Test block execution

Prover offers a method to quickly simulate the execution of certain blocks. It will assist in generating Proof of Blocks and invoke the prove method.

**Note, this method can only be used in a dev environment.**
```bash
# generate the proof of execution for the block 3230626 ~ 3230691
> curl http://localhost:18400 -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"report","params":["3230626", "3230691"]}'
{
	"id": 1,
	"jsonrpc": "2.0",
	"result": {
		"batch_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
		"new_state_root": "0xb86f8091bf5f9be927a6506072ffd4bddb964d9d2ba96561bc759d1e59502d60",
		"prev_state_root": "0xe12b7fd47b9ad6514d4cab6d2538c36a238e553a1536781735414e532ac19a35",
		"signature": "0x71d43a1148942cc75a5f25367efcddc9b907bc2a3a3d04d133ff8d99b8e03f83190792ec704492a7a01a268fe32238d7cd294f3462040f3c7791c9a04f5d721d01",
		"state_hash": "0xf79197d82e0f355652f7266200d7d56a0f74d1a3708a0fd98b63d31efe7a8088",
		"withdrawal_root": "0x0000000000000000000000000000000000000000000000000000000000000000"
	}
}
```