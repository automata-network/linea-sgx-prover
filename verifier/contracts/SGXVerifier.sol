// SPDX-License-Identifier: MIT
pragma solidity ^0.8.9;

import {IAttestation} from "./interfaces/IAttestation.sol";

contract SGXVerifier {
    
    event ProverApproved(address prover);
    event CommitBatch(uint256 indexed batchIndex, bytes32 indexed batchHash, bytes32 prevStateRoot, bytes32 newStateRoot);

    address public owner;
    mapping(address => uint256) public attestedProvers; // prover's pubkey => attestedTime
    mapping(bytes32 => BatchInfo) public batches;

    uint256 public attestValiditySeconds = 3600;

    IAttestation public immutable dcapAttestation;
    uint256 public immutable layer2ChainId;

    struct BatchInfo {
        uint256 batchIndex;
        bytes32 newStateRoot;
        bytes32 prevStateRoot;
        bytes32 withdrawalRoot;
    }

    struct Report {
        uint approved;
        address prover;
        uint blockNumber;
        mapping(address => uint) votes; // 0: unvoted, 1: approve, 2: reject
    }

    uint256 public threshold = 1; // This is an example threshold. Adjust this value as needed.

    modifier onlyOwner() {
        require(msg.sender == owner, "Not authorized");
        _;
    }
    
    constructor(address attestationAddr, uint256 _chainId) {
        owner = msg.sender;
        dcapAttestation = IAttestation(attestationAddr);
        layer2ChainId = _chainId;
    }
    
    function changeOwner(address _newOwner) public onlyOwner {
        owner = _newOwner;
    }

    function changeAttestValiditySeconds(uint256 val) public onlyOwner {
        attestValiditySeconds = val;
    }

    function submitAttestationReport(address prover, bytes calldata reportBytes) public {
        checkAttestation(prover, reportBytes);
        attestedProvers[prover] = block.timestamp;
        emit ProverApproved(prover);
    }

    function commitBatch(uint256 batchId, bytes memory poe) public {
        (bytes32 batchHash, bytes32 stateHash, bytes32 prevStateRoot, bytes32 newStateRoot, bytes32 withdrawalRoot, bytes memory sig) = abi.decode(poe, (bytes32, bytes32, bytes32, bytes32, bytes32, bytes));
        (bytes32 r, bytes32 s, uint8 v) = splitSignature(sig);
        bytes32 msgHash = keccak256(abi.encode(layer2ChainId, batchHash, stateHash, prevStateRoot, newStateRoot, withdrawalRoot, new bytes(65)));
        address signer = ecrecover(msgHash, v, r, s);
        require(signer != address(0), "ECDSA: invalid signature");
        require(attestedProvers[signer] != 0, "Prover not attested");
        require(attestedProvers[signer] + attestValiditySeconds > block.timestamp, "Prover out-of-dated");
        require(batches[batchHash].newStateRoot == bytes32(0), "batch already commit");
        batches[batchHash].batchIndex = batchId;
        batches[batchHash].newStateRoot = newStateRoot;
        batches[batchHash].prevStateRoot = prevStateRoot;
        batches[batchHash].withdrawalRoot = withdrawalRoot;
        emit CommitBatch(batchId, batchHash, prevStateRoot, newStateRoot);
    }

    function recoverPoe(bytes memory poe) view public returns (address) {
        (bytes32 batchHash, bytes32 stateHash, bytes32 prevStateRoot, bytes32 newStateRoot, bytes32 withdrawalRoot, bytes memory sig) = abi.decode(poe, (bytes32, bytes32, bytes32, bytes32, bytes32, bytes));
        (bytes32 r, bytes32 s, uint8 v) = splitSignature(sig);
        bytes32 msgHash = keccak256(abi.encode(layer2ChainId, batchHash, stateHash, prevStateRoot, newStateRoot, withdrawalRoot, new bytes(65)));
        address signer = ecrecover(msgHash, v, r, s);
        require(signer != address(0), "ECDSA: invalid signature");
        return signer;
    }

    function splitSignature(bytes memory sig) internal pure returns (bytes32 r, bytes32 s, uint8 v) {
        require(sig.length == 65, "invalid signature length");

        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
            v := byte(0, mload(add(sig, 96)))
        }

        if (v < 27) {
            v += 27;
        }
        
        require(v == 27 || v == 28, "invalid v value");
    }

    function verifyMrEnclave(bytes32 _mrenclave) view public returns (bool) {
        return dcapAttestation.verifyMrEnclave(_mrenclave);
    }

    function verifyMrSigner(bytes32 _mrsigner) view public returns (bool) {
        return dcapAttestation.verifyMrSigner(_mrsigner);
    }

    function checkAttestation(address prover, bytes calldata reportBytes) view public {
        (bool succ, bytes memory reportData) = dcapAttestation.verifyAttestation(reportBytes);
        require(succ, "attestation validation failed");
        address expectedProver = bytes64ToAddress(reportData);
        require(expectedProver == prover, "attestation prover mismatch");
    }

    function verifyAttestation(address prover, bytes calldata data) view public returns (bool) {
        (bool succ, bytes memory reportData) = dcapAttestation.verifyAttestation(data);
        if (!succ) {
            return false;
        }

        address expectedProver = bytes64ToAddress(reportData);
        if (expectedProver != prover) {
            return false;
        }

        return true;
    }

    function bytes64ToAddress(bytes memory b) private pure returns (address) {
        require(b.length >= 64, "Bytes array too short");

        uint160 addr;
        assembly {
            addr := mload(add(b, 64))
        }
        return address(addr);
    }
}